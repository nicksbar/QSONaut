# Third-Party Logging Software Protocol Research

## Purpose  
Document network protocol patterns used by external logging software for interoperability.

## Research Approach
- Observe network traffic patterns
- Document message formats and state transitions
- No reverse engineering of binaries or inclusion of proprietary assets

## Methodology
1. Set up network capture tools
2. Identify TCP/UDP ports used by logging applications
3. Capture and analyze network traffic patterns
4. Document message structure and protocol behavior

## Key Findings
- Common Windows logging software uses TCP for multi-station networking
- Messages use tag-delimited XML-like format with UTF-16LE encoding
- Typical message types observed:
	- BAMS (station/band/mode)
	- NTWK OPEN
	- WHO
	- MESG (chat message)
	- NTWK CHECK

## Documentation
- Full protocol specification: docs/protocols-n3fjp.md
- Protocol summary: docs/protocol-summary.md
