# N3FJP Protocol Analysis - Real Capture

## Key Findings

### 1. Protocol Format - Wide Characters (UCS-2/UTF-16 LE)
The hex pattern `3c 00` repeats - this is **wide character encoding** (every other byte is null):
- `3c 00` = `<` in UTF-16 LE (Little Endian)
- `42 00` = `B`
- `4f 00` = `O`
- `52 00` = `R`
- `3e 00` = `>`

**Important**: The protocol uses UTF-16 LE, NOT ASCII or UTF-8. Each character takes 2 bytes.

### 2. Message Structure - XML-like Tagging
Messages follow an XML pattern with tags:
```
<BOR>       (Beginning of Record)
<BAMS>      (Band And Mode Status?)
<STATION>   (Station name)
<BAND>      (Band number: 15, 2190, 630, 160, 80, 40, 20, 10, 6, etc)
<MODE>      (CW, DIG, etc)
</STATION>
</BAMS>
<EOR>       (End of Record)
03 04 07    (Possible frame terminators/control bytes)
```

### 3. Detected Message Types

#### BAMS (Band And Mode Status) - Regular Updates
Sent repeatedly by both client and server. Example decoded:
```
<BOR>
<BAMS>
  <STATION>DESKTOP-2HMBTAB</STATION>
  <BAND>15</BAND>
  <MODE>CW</MODE>
</BAMS>
<EOR>
```

Bands observed: 15, 2190, 630, 160, 80, 40, 20, 10, 6 meters

#### NTWK/CHECK - Network Check
```
<BOR>
<NTWK>
<CHECK>
</NTWK>
<EOR>
[control bytes]
```

#### NTWK/FROM - Station List
```
<BOR>
<NTWK>
<FROM> N7UF</FROM>
<TRANSACTION>CHECK</TRANSACTION>
<XMLDATA>
  <COUNT>0</COUNT>
  <LAST>0</LAST>
</XMLDATA>
</NTWK>
<EOR>
[control bytes]
```

#### NTWK/OPEN - Open Transaction
```
<BOR>
<NTWK>
<OPEN>
[possibly contest ID or transaction ID]
</NTWK>
<EOR>
```

### 4. Flow Pattern

1. **Client connects and sends STATION** with current band/mode
2. **Server responds** with echoed STATION
3. **Server sends NTWK/TRANSACTION CLEAR** and **LIST**
4. **Server sends WHO** to get operator info (possibly multicast/broadcast)
5. **Periodic BAMS updates** every ~1-2 seconds while active
6. **Periodic CHECK messages** (~30-60 second intervals)
7. **Server responds with FROM** and list of other stations
8. **FROM/XMLDATA** contains COUNT and LAST fields (for log data?)

### 5. About Contest ID / Dynamic Message Types

**Good news**: The protocol doesn't appear to hard-code contest IDs. Instead:
- Messages are **generic XML tags**
- Band/mode values are **sent dynamically** in BAND and MODE fields
- The protocol appears **contest-agnostic** - just transmits what's in the STATION record
- No contest-specific message types observed

**The issue you mentioned**: If N3FJP has multiple contest apps, they likely:
1. Use the same protocol (what we captured)
2. Only differ in UI and logging database
3. The relay server handles the protocol regardless of which N3FJP app connects

### 6. What We DON'T See (Yet)

- Log entry exchange (NTWK/FROM messages are brief)
- Detailed contest data
- QSO/contact records
- Scoring or multiplier info
- Any contest-ID in the messages

This suggests log data might be:
- Requested separately
- Sent in larger NTWK packets (we saw some 1600+ byte packets)
- Using a different mechanism (database sync?)

## Implications for Implementation

### 1. Don't Hard-Code Contest Data
The protocol is **generic**. Instead of supporting "FD 2024" vs "VHF Contest 2024":
- Accept station records with any BAND/MODE values
- Store whatever is sent
- Let contests define their own band/mode sets in configuration

### 2. Use JSON Configuration
Instead of N3FJP app detection:
```json
{
  "contests": {
    "fieldday": {
      "name": "Field Day",
      "bands": ["160", "80", "40", "20", "15", "10", "6", "2", "70cm"],
      "modes": ["CW", "DIG"],
      "maxClasses": 1
    },
    "vhf_contest": {
      "name": "VHF Contest",
      "bands": ["50", "144", "222", "432"],
      "modes": ["USB", "LSB", "CW", "FM"]
    }
  }
}
```

### 3. Send Configuration to Client
When a client first connects with `NTWK/OPEN`:
- Respond with which contest configuration to use
- Let the client filter bands/modes in its UI
- You don't need to know which N3FJP app it is

### 4. Handle Gracefully
Your server should:
- Accept any BAND/MODE combination
- Store it as-is
- Let database queries filter what's valid for the current contest
- If invalid band for contest, flag in UI but don't reject

## Next Steps

1. **Decode the large 1600+ byte messages** to see QSO/log structure
2. **Implement UTF-16 LE encoding** in your relay/parser
3. **Build flexible band/mode configuration** system
4. **Test with multiple N3FJP apps** to confirm they send same protocol

The relay is working perfectly! You're capturing everything needed.
