# PSK Reporter

RigForge can opt in to reporting received stations to PSK Reporter. Reporting
is disabled by default and requires a real operator callsign and grid.

Enable **Report decoded stations to PSK Reporter** in Operator Profile. Station
Health then shows whether reporting is off, waiting for identity, armed, or in
an error state, together with queued and sent counts.

The reporter follows the service's IPFIX/UDP protocol:

- destination `report.pskreporter.info:4739`;
- receiver identity in every datagram;
- record templates in the first three packets and periodically thereafter;
- automatically extracted sender callsigns with dial plus audio frequency;
- decoder-measured SNR and a decoded sender locator when present;
- five-byte frequency fields, including operation above 4 GHz;
- one report per decoded callsign per five-minute period;
- randomized five-minute batching over one persistent UDP source socket;
- no network work on decoder or GUI threads.

RigForge uses the modern WSJT-X-style sender template: callsign, five-byte
frequency, SNR, ADIF mode/submode, optional sender locator, information source,
and reception time. It intentionally omits iMD. Classic iMD measures
third-order products in modes such as PSK31 and is not a meaningful quality
metric for FT8/FT4 and the other WSJT-family waveforms currently decoded here.

Protocol reference: <https://pskreporter.info/pskdev.html>
