//! Exact-geometry occupancy for the router.
//!
//! The router works on an integer grid, but clearance must be judged in real
//! millimetres. Stamping "pad radius plus clearance" into whole grid cells
//! rounds every obstacle up to the next cell on all four sides, which at a
//! 2.54 mm pad pitch leaves no legal channel at all: a 1.8 mm pad would reserve
//! a whole 3 mm cell instead of its 2.4 mm disc.
//!
//! So obstacles are kept as shapes and distance-tested directly. Bucketing by a
//! coarse spatial grid keeps that cheap: a query only looks at the shapes in the
//! neighbouring buckets.

use crate::board::Board;
use std::collections::{HashMap, HashSet};

/// One thing copper must keep its distance from.
#[derive(Debug, Clone)]
enum Shape {
    /// Round pad or hole: centre, radius, owning net.
    Disc {
        x: f64,
        y: f64,
        r: f64,
        net: usize,
    },
    /// Straight copper segment: endpoints, half width, owning net.
    Segment {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        half: f64,
        net: usize,
    },
}

impl Shape {
    fn net(&self) -> usize {
        match self {
            Shape::Disc { net, .. } | Shape::Segment { net, .. } => *net,
        }
    }

    /// Distance from `p` to this shape's surface (negative inside).
    fn distance(&self, p: (f64, f64)) -> f64 {
        match self {
            Shape::Disc { x, y, r, .. } => ((p.0 - x).powi(2) + (p.1 - y).powi(2)).sqrt() - r,
            Shape::Segment {
                x1,
                y1,
                x2,
                y2,
                half,
                ..
            } => segment_distance(p, (*x1, *y1), (*x2, *y2)) - half,
        }
    }

    /// Bounding box, as `(x0, y0, x1, y1)`.
    fn bounds(&self) -> (f64, f64, f64, f64) {
        match self {
            Shape::Disc { x, y, r, .. } => (x - r, y - r, x + r, y + r),
            Shape::Segment {
                x1,
                y1,
                x2,
                y2,
                half,
                ..
            } => (
                x1.min(*x2) - half,
                y1.min(*y2) - half,
                x1.max(*x2) + half,
                y1.max(*y2) + half,
            ),
        }
    }
}

/// Net index used for copper or a hole that belongs to no net.
pub const NO_NET: usize = usize::MAX;

/// Bucket size in millimetres. Must stay well above the largest clearance query
/// so that a 3×3 neighbourhood always contains every relevant shape.
const BUCKET_MM: f64 = 4.0;

/// Everything on the board copper must clear.
#[derive(Debug, Clone)]
pub struct Occupancy {
    shapes: Vec<Shape>,
    buckets: HashMap<(i32, i32), Vec<usize>>,
    pub width_mm: f64,
    pub height_mm: f64,
    /// Copper-to-outline clearance the process requires.
    edge_mm: f64,
}

impl Occupancy {
    /// Collect pads, mounting holes and mechanical keep-outs for `board`.
    ///
    /// `tracks` seeds copper routed earlier in the same run or imported from a
    /// file; each entry is `(x1, y1, x2, y2, half, net)`.
    pub fn new(board: &Board, tracks: &[(f64, f64, f64, f64, f64, usize)]) -> Self {
        let mut shapes = Vec::new();

        for (pos, size, net_name) in board.all_pads() {
            let net = net_name
                .as_deref()
                .and_then(|n| board.nets.iter().position(|b| b.name == n))
                .unwrap_or(NO_NET);
            shapes.push(Shape::Disc {
                x: pos.0,
                y: pos.1,
                r: size / 2.0,
                net,
            });
        }

        // A mounting hole is not copper: the hole wall plus a full clearance
        // band is off-limits to every net.
        for hole in &board.mounting_holes {
            shapes.push(Shape::Disc {
                x: hole.x,
                y: hole.y,
                r: hole.drill_mm / 2.0,
                net: NO_NET,
            });
        }

        for &(x1, y1, x2, y2, half, net) in tracks {
            shapes.push(Shape::Segment {
                x1,
                y1,
                x2,
                y2,
                half,
                net,
            });
        }

        let mut occupancy = Self {
            shapes,
            buckets: HashMap::new(),
            width_mm: board.width_mm,
            height_mm: board.height_mm,
            edge_mm: board.edge_clearance_mm(),
        };
        occupancy.rebuild_buckets();
        occupancy
    }

    /// Add routed copper so later nets must clear it.
    pub fn add_track(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, half: f64, net: usize) {
        self.shapes.push(Shape::Segment {
            x1,
            y1,
            x2,
            y2,
            half,
            net,
        });
        // Index only the new shape: rebuilding everything after every segment
        // would make a full board quadratic.
        let last = self.shapes.len() - 1;
        self.index(last);
    }

    fn rebuild_buckets(&mut self) {
        self.buckets.clear();
        for index in 0..self.shapes.len() {
            self.index(index);
        }
    }

    fn index(&mut self, index: usize) {
        let (x0, y0, x1, y1) = self.shapes[index].bounds();
        let (cx0, cy0) = bucket_of((x0, y0));
        let (cx1, cy1) = bucket_of((x1, y1));
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                self.buckets.entry((cx, cy)).or_default().push(index);
            }
        }
    }

    /// May a straight centre-line segment be laid for `net` at `half` width?
    ///
    /// Every 0.1 mm along the segment is tested with the same exact-mm
    /// clearance as `allows`. Used by the line-of-sight path simplifier and by
    /// corner chamfers, where a few endpoint samples are not enough.
    pub fn allows_segment(
        &self,
        a: (f64, f64),
        b: (f64, f64),
        half: f64,
        clearance: f64,
        net: usize,
    ) -> bool {
        let length = (b.0 - a.0).hypot(b.1 - a.1);
        let steps = (length / 0.2).ceil().max(1.0) as usize;
        (0..=steps).all(|i| {
            let t = i as f64 / steps as f64;
            self.allows(
                (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t),
                half,
                clearance,
                net,
            )
        })
    }

    /// May a track of net `net`, `half` millimetres wide on each side of its
    /// centre line, pass through `p`?
    ///
    /// `clearance` is the process minimum copper-to-copper gap. Copper of the
    /// same net is not an obstacle: a net may touch itself.
    pub fn allows(&self, p: (f64, f64), half: f64, clearance: f64, net: usize) -> bool {
        // Keep the copper, not just the centre line, `edge_mm` inside the board.
        let margin = self.edge_mm + half;
        if p.0 < margin
            || p.1 < margin
            || p.0 > self.width_mm - margin
            || p.1 > self.height_mm - margin
        {
            return false;
        }

        let need = clearance + half;
        let (bx, by) = bucket_of(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(indices) = self.buckets.get(&(bx + dx, by + dy)) else {
                    continue;
                };
                for &i in indices {
                    let shape = &self.shapes[i];
                    if shape.net() == net {
                        continue;
                    }
                    if shape.distance(p) < need {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Slack between the required and the available gap at `p`.
    ///
    /// Returns how much extra room a track of width `2 * half` would have beyond
    /// `clearance`, capped at `cap_mm`. Zero means it exactly fits, and a large
    /// value means open board. The router uses it to prefer roomy channels: a
    /// path that hugs every obstacle takes the shortest route but blocks the nets
    /// routed after it, which is what makes a single-layer board fail to close.
    pub fn slack(&self, p: (f64, f64), half: f64, clearance: f64, net: usize, cap_mm: f64) -> f64 {
        let need = clearance + half;
        let mut best = cap_mm;
        let (bx, by) = bucket_of(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(indices) = self.buckets.get(&(bx + dx, by + dy)) else {
                    continue;
                };
                for &i in indices {
                    let shape = &self.shapes[i];
                    if shape.net() == net {
                        continue;
                    }
                    let slack = shape.distance(p) - need;
                    if slack < best {
                        best = slack;
                    }
                    if best <= 0.0 {
                        return 0.0;
                    }
                }
            }
        }
        best
    }

    /// Nets whose copper bounds the free region reachable from `p`.
    ///
    /// Flood-fills the cells a track of `net` could occupy starting at `p`, then
    /// reports every foreign net with copper close enough to the region boundary
    /// to be part of the wall around it. Those are the nets that have to be
    /// ripped up for `net` to have any chance. The region size is returned too:
    /// a tiny region means the terminal really is boxed in.
    pub fn blocking_nets(
        &self,
        p: (f64, f64),
        half: f64,
        clearance: f64,
        net: usize,
        grid_mm: f64,
        cell_cap: usize,
    ) -> (usize, Vec<usize>) {
        use std::collections::VecDeque;

        let cell_of = |q: (f64, f64)| {
            (
                (q.0 / grid_mm).round() as i32,
                (q.1 / grid_mm).round() as i32,
            )
        };
        let point_of = |c: (i32, i32)| (c.0 as f64 * grid_mm, c.1 as f64 * grid_mm);

        let start = cell_of(p);
        let mut seen: HashSet<(i32, i32)> = HashSet::new();
        let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
        seen.insert(start);
        queue.push_back(start);
        let mut blockers: HashSet<usize> = HashSet::new();

        // Copper within this distance of a wall is close enough to be the wall.
        let reach = clearance + half + grid_mm;

        while let Some(c) = queue.pop_front() {
            if seen.len() > cell_cap {
                break;
            }
            for next in [
                (c.0 + 1, c.1),
                (c.0 - 1, c.1),
                (c.0, c.1 + 1),
                (c.0, c.1 - 1),
            ] {
                if seen.contains(&next) {
                    continue;
                }
                let q = point_of(next);
                if self.allows(q, half, clearance, net) {
                    seen.insert(next);
                    queue.push_back(next);
                } else {
                    for owner in self.owners_within(q, reach, net) {
                        blockers.insert(owner);
                    }
                }
            }
        }

        let mut out: Vec<usize> = blockers.into_iter().collect();
        out.sort_unstable();
        (seen.len(), out)
    }

    /// Every foreign net with copper within `radius` of `p`.
    pub fn owners_within(&self, p: (f64, f64), radius: f64, net: usize) -> Vec<usize> {
        let (bx, by) = bucket_of(p);
        let span = ((radius / BUCKET_MM).ceil() as i32).max(1);
        let mut out = Vec::new();
        for dx in -span..=span {
            for dy in -span..=span {
                let Some(indices) = self.buckets.get(&(bx + dx, by + dy)) else {
                    continue;
                };
                for &i in indices {
                    let shape = &self.shapes[i];
                    let owner = shape.net();
                    if owner == net || owner == NO_NET {
                        continue;
                    }
                    if shape.distance(p) <= radius && !out.contains(&owner) {
                        out.push(owner);
                    }
                }
            }
        }
        out
    }

    /// Net index of the closest foreign copper within `radius` of `p`.
    ///
    /// Used for the audio-isolation cost: running a signal track alongside a
    /// power rail is legal but should be avoided.
    pub fn foreign_within(&self, p: (f64, f64), radius: f64, net: usize) -> Option<usize> {
        let (bx, by) = bucket_of(p);
        let span = ((radius / BUCKET_MM).ceil() as i32).max(1);
        let mut best: Option<(f64, usize)> = None;
        for dx in -span..=span {
            for dy in -span..=span {
                let Some(indices) = self.buckets.get(&(bx + dx, by + dy)) else {
                    continue;
                };
                for &i in indices {
                    let shape = &self.shapes[i];
                    let owner = shape.net();
                    if owner == net || owner == NO_NET {
                        continue;
                    }
                    let d = shape.distance(p);
                    if d <= radius && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, owner));
                    }
                }
            }
        }
        best.map(|(_, owner)| owner)
    }
}

fn bucket_of(p: (f64, f64)) -> (i32, i32) {
    (
        (p.0 / BUCKET_MM).floor() as i32,
        (p.1 / BUCKET_MM).floor() as i32,
    )
}

/// Shortest distance from `p` to the segment `a`–`b`.
fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-12 {
        return ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt();
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len_sq).clamp(0.0, 1.0);
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, Net, NetClass, Pad, PadRef};

    #[test]
    fn segment_distance_is_correct() {
        assert!((segment_distance((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)) - 3.0).abs() < 1e-9);
        assert!((segment_distance((-4.0, 0.0), (0.0, 0.0), (10.0, 0.0)) - 4.0).abs() < 1e-9);
        assert!((segment_distance((3.0, 4.0), (0.0, 0.0), (0.0, 0.0)) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_gap_wide_enough_for_a_track_is_allowed() {
        // Two pads 2.54 mm apart, radius 0.9 mm: 0.74 mm between the edges.
        let mut board = Board::new("gap", 40.0, 20.0, LutPreset::reliable());
        let mut fp = Footprint::new("J1", "HDR", NetClass::Signal, 10.0, 10.0, 8.0, 6.0);
        fp.pads = vec![
            Pad::new(1.0, 3.0, 1.8, 1.0),
            Pad::new(3.54, 3.0, 1.8, 1.0),
        ];
        board.add_footprint(fp);

        let occupancy = Occupancy::new(&board, &[]);
        let clearance = board.preset.min_clearance_mm;
        // Midway between the pads: 1.27 mm from each centre, 0.37 mm from each
        // edge. A 0.4 mm track needs 0.2 + 0.3 = 0.5 mm, so it must not fit.
        let middle = (12.27, 13.0);
        assert!(
            !occupancy.allows(middle, 0.2, clearance, 99),
            "a 0.4 mm track must not squeeze between 1.8 mm pads on a 2.54 mm pitch"
        );
        // A hairline track does fit at the same place.
        assert!(
            occupancy.allows(middle, 0.01, 0.05, 99),
            "a thin test track should fit in the same gap"
        );
    }

    #[test]
    fn a_nets_own_copper_is_not_an_obstacle() {
        let mut board = Board::new("own", 40.0, 20.0, LutPreset::reliable());
        let mut fp = Footprint::new("R1", "10k", NetClass::Signal, 10.0, 10.0, 12.0, 6.0);
        fp.pads = vec![
            Pad::new(1.0, 3.0, 1.8, 1.0),
            Pad::new(11.0, 3.0, 1.8, 1.0),
        ];
        board.add_footprint(fp);
        board.add_net(Net::new(
            "N",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R1", 1)],
        ));

        let occupancy = Occupancy::new(&board, &[]);
        // Straight on top of the pad centre, for the pad's own net.
        assert!(occupancy.allows((11.0, 13.0), 0.2, 0.3, 0));
    }

    #[test]
    fn mounting_holes_and_the_outline_are_off_limits() {
        let mut board = Board::new("mech", 40.0, 30.0, LutPreset::reliable());
        board.add_mounting_hole(crate::board::MountingHole::new(5.0, 5.0, 3.2));
        let occupancy = Occupancy::new(&board, &[]);

        assert!(
            !occupancy.allows((5.0, 5.0), 0.2, 0.3, 0),
            "copper must not enter a mounting hole"
        );
        assert!(
            !occupancy.allows((0.2, 15.0), 0.2, 0.3, 0),
            "copper must stay clear of the board edge"
        );
        assert!(
            occupancy.allows((20.0, 15.0), 0.2, 0.3, 0),
            "the middle of the board is free"
        );
    }

    #[test]
    fn existing_tracks_become_obstacles() {
        let mut board = Board::new("trk", 40.0, 30.0, LutPreset::reliable());
        board.add_footprint(Footprint::new(
            "R1",
            "10k",
            NetClass::Signal,
            2.0,
            2.0,
            6.0,
            4.0,
        ));
        let mut occupancy = Occupancy::new(&board, &[]);
        occupancy.add_track(5.0, 15.0, 35.0, 15.0, 0.2, 7);

        assert!(
            !occupancy.allows((20.0, 15.0), 0.2, 0.3, 0),
            "another net's track blocks the point it runs through"
        );
        assert!(
            occupancy.allows((20.0, 18.0), 0.2, 0.3, 0),
            "3 mm away is clear of a 0.4 mm track"
        );
        assert!(
            occupancy.allows((20.0, 15.0), 0.2, 0.3, 7),
            "the same net may cross its own copper"
        );
    }

    #[test]
    fn foreign_copper_is_reported_with_its_net() {
        let mut board = Board::new("near", 40.0, 30.0, LutPreset::reliable());
        board.add_footprint(Footprint::new(
            "R1",
            "10k",
            NetClass::Power,
            10.0,
            10.0,
            6.0,
            4.0,
        ));
        board.add_net(Net::new(
            "V+",
            NetClass::Power,
            vec![PadRef::new("R1", 0), PadRef::new("R1", 1)],
        ));
        let occupancy = Occupancy::new(&board, &[]);
        assert_eq!(occupancy.foreign_within((12.0, 14.0), 2.0, 5), Some(0));
        assert_eq!(occupancy.foreign_within((12.0, 14.0), 2.0, 0), None);
    }
}
