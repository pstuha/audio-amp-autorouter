//! AmpRoute MCP server — JSON-RPC 2.0 over stdio.
//!
//! One request per line on stdin, one response per line on stdout. Nothing else
//! is ever written to stdout, so an agent can parse it as a clean stream.
//! Diagnostics go to stderr. See `docs/agent-api.md` for the tool contract.

use amproute_studio::agent::AgentService;
use std::io::{self, BufRead, Write};

fn main() {
    let mut service = AgentService::default();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("stdin error: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = service.handle_line(&line);
        if let Err(e) = writeln!(out, "{response}").and_then(|_| out.flush()) {
            eprintln!("stdout error: {e}");
            break;
        }
    }
}
