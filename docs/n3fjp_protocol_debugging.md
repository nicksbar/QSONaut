# N3FJP Protocol Debugging Guide

## Overview
This guide helps you debug the N3FJP protocol implementation using a MITM (Man-In-The-Middle) relay to capture and analyze bidirectional communication between client and server.

## Quick Start

### 1. Start the MITM Relay
The relay listens for client connections and forwards them to the real server while recording all traffic.

```bash
# Terminal 1: Start the relay
# Listens on port 2000, forwards to localhost:1000
npm run scripts:relay

# Or manually:
npm run build
npx ts-node scripts/n3fjp_mitm_relay.ts 2000 localhost 1000
```

### 2. Start the N3FJP Server
In another terminal, start the actual N3FJP server on port 1000.

```bash
# Terminal 2: Start server
# Using the N3FJP stub server
node scripts/n3fjp_server_stub.py

# Or use any N3FJP-compatible server
```

### 3. Run Your Client
Configure your client to connect to the relay instead of the server.

```bash
# Terminal 3: Run your client
# Instead of: client.connect('localhost', 1000)
# Use: client.connect('localhost', 2000)

npm run dev:all
```

## What Gets Recorded

The relay captures:
- **Log file** (`captures/n3fjp_mitm_TIMESTAMP.log`): Human-readable log with timestamps
- **Data file** (`captures/n3fjp_mitm_TIMESTAMP.json`): Structured data with hex/ASCII for analysis

Example log entry:
```
[2026-02-01T15:30:45.123Z] [conn-1] Client connected from 127.0.0.1:54321
[2026-02-01T15:30:45.234Z] [conn-1] Connected to server at localhost:1000
[2026-02-01T15:30:45.345Z] [conn-1] client→server (25 bytes): GETCONTESTNUMBER\r\n
[2026-02-01T15:30:45.456Z] [conn-1] server→client (50 bytes): CONTESTNUMBER,1,2024 ...
```

## Analyzing Captured Data

### View the Log File
```bash
tail -f captures/n3fjp_mitm_*.log
```

### Parse the Data File
```bash
npx ts-node scripts/analyze_n3fjp_log.ts captures/n3fjp_mitm_*.json
```

This shows:
- All client commands sent
- All server responses received
- Hex and ASCII representations
- Message counts and timing

## Debugging "Attempting to Connect" Issues

If your client stays on "attempting to connect", check:

### 1. Network connectivity
```bash
# Check if you can reach the relay
nc -zv localhost 2000

# Check if server is running
nc -zv localhost 1000
```

### 2. Review the relay logs
- Did client connect to relay? (look for "Client connected")
- Did relay connect to server? (look for "Connected to server")
- Any errors? (look for "error:")

### 3. Check the captured messages
- Is client sending any data? (client→server messages)
- Is server responding? (server→client messages)
- What's the exact content (hex and ASCII)?

### 4. Compare with expected protocol
- Are commands in the right format?
- Are responses what you expect?
- Are there protocol version mismatches?

## Common Issues

### "Client connection closed immediately"
- Server not responding to initial handshake
- Protocol mismatch (client/server versions differ)
- Check captured messages for what server sent

### "Server connection error: Connection refused"
- Server not running on port 1000
- Server crashed or not started
- Port conflicts (check `netstat -tulpn | grep :1000`)

### "No messages in log"
- Connection established but no data sent
- Client hanging before sending initial command
- Check UI logs for what client is trying to do

## Port Configuration

Default:
- **Client connects to**: `localhost:2000` (relay)
- **Relay forwards to**: `localhost:1000` (server)

Change via command line:
```bash
npx ts-node scripts/n3fjp_mitm_relay.ts 3000 192.168.1.100 1000
# Now client connects to localhost:3000, relay to 192.168.1.100:1000
```

## Protocol References

### Initial Handshake (Expected)
```
Client:  GETCONTESTNUMBER
Server:  CONTESTNUMBER,<number>,<year>
         
Client:  GETSERVERVERSION
Server:  SERVERVERSION,<version>

Client:  GETOPERATORCALL
Server:  OPERATORCALL,<callsign>
```

### Examining Hex Data
When analyzing captured hex:
- `0d` = carriage return (`\r`)
- `0a` = line feed (`\n`)
- `2c` = comma (`,`)
- `00` = null terminator
- Readable ASCII is shown alongside

Example:
```
HEX: 47 45 54 43 4f 4e 54 45 53 54 4e 55 4d 42 45 52 0d 0a
ASCII: GETCONTESTNUMBER\r\n
```

## Files Generated

Each relay session creates:
- **MITM relay log**: `captures/n3fjp_mitm_TIMESTAMP.log`
- **MITM relay data**: `captures/n3fjp_mitm_TIMESTAMP.json`

Keep these for protocol analysis and debugging.

## Next Steps

1. **Capture baseline**: Run with known working N3FJP server
2. **Compare**: See what real server sends vs. your stub
3. **Fix**: Update stub or client to match
4. **Test**: Verify client gets past "attempting to connect"
5. **Document**: Add discovered protocol details to docs/

## Useful Tools

```bash
# Monitor relay in real-time
tail -f captures/n3fjp_mitm_*.log

# Analyze after session ends
npx ts-node scripts/analyze_n3fjp_log.ts captures/n3fjp_mitm_*.json

# Count messages by type
grep -o "client→server\|server→client" captures/n3fjp_mitm_*.log | sort | uniq -c

# Find specific commands
grep "GETCONTESTNUMBER\|GETSERVERVERSION" captures/n3fjp_mitm_*.log
```

## Troubleshooting

**Q: Relay won't start**
```
A: Check if ports are already in use
   lsof -i :2000 (check client port)
   lsof -i :1000 (check server port)
```

**Q: No data in logs**
```
A: Connection established but nothing sent
   - Check client implementation (is it sending commands?)
   - Check if client is waiting for server greeting first
   - Look at relay log for what server sent
```

**Q: Can't parse JSON logs**
```
A: Log file might be incomplete if relay crashed
   - Check relay log file for errors
   - Try to parse what's there: npx ts-node scripts/analyze_n3fjp_log.ts
```
