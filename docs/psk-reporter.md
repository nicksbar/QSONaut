# PSK Reporter

QSONaut can opt in to reporting received stations to PSK Reporter. Reporting
is disabled by default and requires a real operator callsign and grid.

Enable **Report decoded stations to PSK Reporter** in Operator Profile. The status bar
then shows whether reporting is off, waiting for identity, armed, or in an error state,
together with queued and sent counts. The Reporting panel also exposes submission-rule
controls so you can follow (or relax) the service's guidance:

- **Batch every** — nominal interval between batches (default 300 s). The actual interval
  is randomized up to +30 s so bursts from many clients don't collide.
- **Re-report same call after** — minimum time before the same callsign is reported again
  (default 300 s), reducing load on the collector.
- **Max pending** — largest number of reports held before a batch is forced out early
  (default 80; WSJT-X uses 2048).

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

QSONaut uses the modern WSJT-X-style sender template: callsign, five-byte
frequency, SNR, ADIF mode/submode, optional sender locator, information source,
and reception time. It intentionally omits iMD. Classic iMD measures
third-order products in modes such as PSK31 and is not a meaningful quality
metric for FT8/FT4 and the other WSJT-family waveforms currently decoded here.

Protocol reference: <https://pskreporter.info/pskdev.html>
