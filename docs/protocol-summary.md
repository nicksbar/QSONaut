# External Logging Protocol Implementation Summary

## Overview
Implementation summary of TCP-based multi-station networking protocol used by Windows ham radio logging applications. Protocol was analyzed through network traffic observation and testing.

## Transport Layer
- **Protocol**: TCP/IP
- **Port**: Configurable (commonly 1000 or 10000)
- **Encoding**: UTF-16LE (little-endian Unicode)
- **Message Framing**: 
  - Start: `<BOR>` (Begin of Record)
  - End: `<EOR>` (End of Record)
  - Trailer: `\x03\x04\x07` (3 control bytes after EOR)
  - Multi-message bundling: Multiple messages may arrive without EOR between them; split on BOR boundaries

## Message Types (7 Core Types)

### 1. BAMS - Band/Mode/Station Announcement
**Direction**: Client → Server (+ broadcast)
**Format**:
```xml
<BOR><BAMS>
  <STATION>N7UF</STATION>
  <BAND>15</BAND>
  <MODE>PH</MODE>
</BAMS><EOR>[trailer]
```
**Fields**:
- STATION: Operator callsign
- BAND: HF band (10, 15, 20, 40, 80, etc.)
- MODE: Operating mode (PH = SSB, CW, DIG = Digital)

**Server Response**: Broadcast to all other connected clients (relay network traffic)

### 2. NTWK OPEN - Network Session Opening
**Direction**: Client → Server
**Format**:
```xml
<BOR><NTWK><OPEN></OPEN></NTWK><EOR>[trailer]
```
**Server Response**:
```xml
<BOR><NTWK><OPEN></OPEN></NTWK><EOR>[trailer]
```

### 3. NTWK CHECK - Heartbeat/Keepalive
**Direction**: Client → Server (periodic)
**Format**:
```xml
<BOR><NTWK><CHECK></CHECK></NTWK><EOR>[trailer]
```
**Server Response**:
```xml
<BOR><NTWK><CHECK></CHECK></NTWK><EOR>[trailer]
```
**Frequency**: Approximately every 30-60 seconds

### 4. NTWK TRANSACTION - Log Entry Submission
**Direction**: Client → Server
**Format**:
```xml
<BOR><NTWK>
  <FROM>hostname</FROM>
  <TRANSACTION>ADD|UPDATE|DELETE</TRANSACTION>
  <XMLDATA>
    <FLDBAND>15</FLDBAND>
    <FLDCALL>W1AW</FLDCALL>
    <FLDCLASS>1H</FLDCLASS>
    <FLDCOMPUTERNAME>LAPTOP-3ROLRHLS</FLDCOMPUTERNAME>
    <FLDCONTESTID>ARRL-FD</FLDCONTESTID>
    <FLDCONTINENT>NA</FLDCONTINENT>
    <FLDCOUNTRYDXCC>291</FLDCOUNTRYDXCC>
    <FLDCOUNTRYWORKED>USA</FLDCOUNTRYWORKED>
    <FLDCQZONE>05</FLDCQZONE>
    <FLDDATESTR>2026/01/31</FLDDATESTR>
    <FLDINITIALS>NAB</FLDINITIALS>
    <FLDITUZONE>08</FLDITUZONE>
    <FLDMODE>PH</FLDMODE>
    <FLDMODECONTEST>PH</FLDMODECONTEST>
    <FLDOPERATOR>N7UF</FLDOPERATOR>
    <FLDPOINTS>0</FLDPOINTS>
    <FLDPREFIX>W1</FLDPREFIX>
    <FLDPRIMARYKEY>1</FLDPRIMARYKEY>
    <FLDQSLR>N</FLDQSLR>
    <FLDQSLS>N</FLDQSLS>
    <FLDSECTION>OR</FLDSECTION>
    <FLDSPCNUM>OR</FLDSPCNUM>
    <FLDSTATE>OR</FLDSTATE>
    <FLDSTATION> N7UF</FLDSTATION>
    <FLDTIMEONSTR>18:07:25</FLDTIMEONSTR>
    <FLDTRANSMITTERID>0</FLDTRANSMITTERID>
  </XMLDATA>
</NTWK><EOR>[trailer]
```

**Server Response** (NEW):
```xml
<BOR><NTWK><TRANSACTION></TRANSACTION></NTWK><EOR>[trailer]
```

**QSO Fields** (see [canonical-log-model.md](canonical-log-model.md) for full mapping):
- Contact info: FLDCALL, FLDBAND, FLDMODE, FLDDATESTR, FLDTIMEONSTR
- Location: FLDCONTINENT, FLDCOUNTRYWORKED, FLDCQZONE, FLDITUZONE, FLDSECTION, FLDSTATE
- Station: FLDSTATION, FLDOPERATOR, FLDCLASS, FLDCOMPUTERNAME
- Contest: FLDCONTESTID, FLDPREFIX, FLDTRANSMITTERID, FLDPOINTS
- QSL: FLDQSLR, FLDQSLS

### 5. WHO - Station Roster Query/Response
**Direction**: Client → Server
**Format** (Client Query):
```xml
<BOR><WHO><EOR>[trailer]
```

**Format** (Server Response):
```xml
<BOR><WHO>
  <STATION>N7UF</STATION>
  <STATION>W1AW</STATION>
  <STATION>K2</STATION>
</WHO><EOR>[trailer]
```

### 6. MESG - Chat Message
**Direction**: Client → Server → All Clients (broadcast)
**Format**:
```xml
<BOR><MESG>
  <TO></TO>
  <FROM>N7UF</FROM>
  <MSGTXT>Hello everyone!</MSGTXT>
</MESG><EOR>[trailer]
```

**Fields**:
- TO: Recipient (empty for broadcast, callsign for direct message)
- FROM: Sender callsign
- MSGTXT: Message text

**Server Behavior**: Broadcast to all connected clients (relay)

### 7. SCLK - Sync Clock
**Direction**: Client → Server
**Format**:
```xml
<BOR><SCLK>
  <YEAR>2026</YEAR>
  <MONTH>01</MONTH>
  <DAY>31</DAY>
  <HOUR>18</HOUR>
  <MINUTE>07</MINUTE>
  <SECOND>25</SECOND>
  <MILLISECOND>123</MILLISECOND>
</SCLK><EOR>[trailer]
```

**Server Response**: Currently no explicit ACK sent (may not be required)

## Client Connection Sequence
```
1. Client connects to server
2. Server sends greeting: <BOR><HELLO>...</HELLO><EOR>
3. Client sends: <BOR><BAMS>...<BAMS><EOR>
4. Client sends: <BOR><NTWK><OPEN></OPEN></NTWK><EOR>
   Server responds: ACK OPEN
5. Client sends: <BOR><WHO><EOR>
   Server responds: WHO with station list
6. Client sends: <BOR><SCLK>...<SCLK><EOR>
   Server may echo or ignore
7. Client sends periodic: <BOR><NTWK><CHECK></CHECK></NTWK><EOR>
   Server responds: ACK CHECK
8. Client may send: <BOR><MESG>...<MESG><EOR>
   Server broadcasts to all
9. Client may send: <BOR><BAMS>...<BAMS><EOR> (band/mode change)
   Server relays to all
10. Client may send: <BOR><NTWK><TRANSACTION>...<XMLDATA>...<XMLDATA></NTWK><EOR>
    Server responds: ACK TRANSACTION
```

## Server Implementation Requirements

### Minimum Viable Server
1. Accept TCP connections on configurable port
2. Send greeting on new connection
3. Parse UTF-16LE messages with BOR/EOR framing
4. Respond to NTWK OPEN with ACK
5. Respond to NTWK CHECK with ACK
6. Respond to WHO with station list (can be empty)
7. Broadcast MESG to all clients
8. Broadcast BAMS to all other clients
9. Accept NTWK TRANSACTION and send ACK

### Optional Enhanced Features
1. Store log entries in database
2. Maintain persistent station list with current band/mode
3. Support multi-network relay (multiple server peers)
4. Implement rig control (CAT integration)
5. ADIF export

## Testing Checklist
- [x] Single client connection and handshake
- [x] BAMS (band/mode) updates
- [x] MESG (chat) messages
- [x] WHO query
- [x] NTWK CHECK heartbeat
- [x] NTWK TRANSACTION (log entry)
- [ ] Multiple concurrent clients
- [ ] Broadcast relay (MESG, BAMS to other clients)
- [ ] Long-duration connection stability (60+ minutes)
- [ ] Network recovery on drop/reconnect

## Known Limitations & Future Work
- SCLK timestamp parsing not fully utilized (sent but not ACKed)
- No persistent log storage in stub
- No multi-network relay between server instances
- No rig control (CAT) integration
- No UDP bridge to N1MM/DXLab/WaveLog yet

## References
- [Canonical Log Model](canonical-log-model.md)
- [N3FJP Protocol Details](protocols-n3fjp.md)
- [Current Relay Implementation](../src/relay.ts)
