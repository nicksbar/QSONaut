# UDP Protocols (Research)

## Target Apps
- N1MM
- DXLab
- N3FJP
- WaveLog

## Notes
- N3FJP Field Day appears to use TCP with tagged messages for local logging sync (not UDP).

## Research Outputs
- Sample payloads
- Field mapping to internal log model
- Required/optional fields

## Interop Strategy
- Define a canonical internal log event model.
- Implement per-format adapters (parse → canonical, canonical → serialize).
- Auto-detect format when possible, else allow per-activity selection.
- Maintain golden fixtures per format for regression tests.

## Next Steps
- Capture payloads from each app.
- Build minimal parser for each format.
- Prioritize at least two formats for MVP if they differ materially.
