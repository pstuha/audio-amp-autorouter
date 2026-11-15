# AmpRoute Studio — simulation report

Board: **unknown**

## Current loops
- Count: 5
- Critical: 3
  - GND: 746 mm², 14.6 nH, CRITICAL
  - MID: 169 mm², 49.4 nH, ok
  - IN: 103 mm², 63.1 nH, ok
  - VCC: 68 mm², 44.8 nH, CRITICAL
  - OUT: 15 mm², 9.2 nH, CRITICAL

## Thermal
- Components analysed: 0

## Audio predictions
- Damping factor: **25.0**
- THD @1kHz (estimate): 0.3286 %
- Slew rate: 40.0 V/µs
- PSRR 100Hz/1kHz: 80/86 dB

## Stability
- Phase margin: **35.0°**
- Gain margin: 12.0 dB
- Capacitive load stable up to: 10 µF
- **Stable: NO**

## IR drop
  - VCC: 0.0 mV
  - GND: 0.0 mV

## Recommendations
- Reduce area of 3 critical current loops (total area: 829.1 mm²)
- Damping factor 25.0 is below ideal (>50). Consider thicker output traces or lower emitter resistance.
- THD estimate 0.3286% is elevated. Review loop areas and decoupling.

## Warnings
- Phase margin 35° may be insufficient