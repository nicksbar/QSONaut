# N3FJP Protocol Analysis Setup - Complete

## What You Got

A complete protocol debugging infrastructure for analyzing N3FJP client-server communication:

### Scripts Created
1. **`scripts/n3fjp_mitm_relay.ts`** (210 lines)
   - Transparent TCP proxy/relay
   - Listens for client connections
   - Forwards to real N3FJP server
   - Records ALL bidirectional traffic to JSON + human-readable log

2. **`scripts/analyze_n3fjp_log.ts`** (96 lines)
   - Parses captured MITM logs
   - Shows command/response patterns
   - Displays hex and ASCII representations
   - Analyzes connection timeline

3. **`scripts/n3fjp_test_setup.sh`** (executable)
   - Orchestrates entire test workflow
   - Commands: `relay`, `server`, `analyze`, `monitor`, `check`
   - Handles port detection and log management

### Documentation Created
1. **`docs/n3fjp_protocol_debugging.md`** - Detailed debugging guide
2. **`docs/n3fjp_testing_guide.md`** - Complete testing workflow
3. **`N3FJP_TEST_QUICK_REF.md`** - Quick reference for copy-paste setup

### NPM Scripts Added
```json
{
  "relay": "ts-node scripts/n3fjp_mitm_relay.ts",
  "relay:default": "ts-node scripts/n3fjp_mitm_relay.ts 2000 localhost 1000"
}
```

## How to Use

### One-Command Start (3 Terminals)

**Terminal 1:**
```bash
cd /home/nick/YAHAML && python3 scripts/n3fjp_server_stub.py
```

**Terminal 2:**
```bash
cd /home/nick/YAHAML && npm run relay:default
```

**Terminal 3:**
```bash
cd /home/nick/YAHAML && npm run dev:all
```

Configure your client to connect to `localhost:2000` (the relay) instead of `localhost:1000`.

### Analysis

```bash
# After running the above, in another terminal:
cd /home/nick/YAHAML
./scripts/n3fjp_test_setup.sh analyze
```

### What You'll See

The analysis output shows:
- Every command sent by client (with hex and ASCII)
- Every response from server (with hex and ASCII)
- Exact timing of each message
- Connection state changes

Example output:
```
[2026-02-01T15:30:45.345Z] [conn-1] client→server (25 bytes)
  HEX: 47 45 54 43 4f 4e 54 45 53 54 4e 55 4d 42 45 52 0d 0a
  ASCII: GETCONTESTNUMBER\r\n

[2026-02-01T15:30:45.456Z] [conn-1] server→client (50 bytes)
  HEX: 43 4f 4e 54 45 53 54 4e 55 4d 42 45 52 2c 31 2c 32 30 32 34 0d 0a
  ASCII: CONTESTNUMBER,1,2024\r\n
```

## Key Features

✅ **Transparent Proxy** - Client connects to relay without code changes  
✅ **Full Traffic Capture** - Every byte in both directions recorded  
✅ **Hex + ASCII** - See exact bytes and readable text  
✅ **Timestamps** - Precise timing of each message  
✅ **JSON Export** - Structured data for programmatic analysis  
✅ **Human Logs** - Readable logs for quick inspection  
✅ **Port Flexibility** - Configure any ports needed  
✅ **Easy Analysis** - Single command to analyze captured data  

## Debugging "Attempting to Connect"

The relay will show you exactly what's happening:

1. **Check if client connects to relay:**
   ```bash
   grep "Client connected" captures/n3fjp_mitm_*.log
   ```

2. **Check if relay connects to server:**
   ```bash
   grep "Connected to server" captures/n3fjp_mitm_*.log
   ```

3. **Check what client sends:**
   ```bash
   grep "client→server" captures/n3fjp_mitm_*.log
   ```

4. **Check what server responds:**
   ```bash
   grep "server→client" captures/n3fjp_mitm_*.log
   ```

If you see "client connected" but "Connected to server" doesn't appear, the relay can't reach your server.

If messages exist in both directions but client still hangs, the protocol handshake must be wrong.

## Files Generated During Testing

Each test session creates:
- `captures/n3fjp_mitm_<timestamp>.log` - Human-readable log
- `captures/n3fjp_mitm_<timestamp>.json` - Structured JSON data

Keep these for:
- Comparing before/after fixes
- Protocol specification documentation
- Future debugging reference

## Verify Setup

Check everything is ready:
```bash
cd /home/nick/YAHAML
./scripts/n3fjp_test_setup.sh check
```

Expected output:
```
=== Port Status ===
[WARNING] Client port 2000 is available
[WARNING] Server port 1000 is available

=== Captured Logs ===
[WARNING] No captured logs found yet
```

If you get `command not found` for lsof, install it:
```bash
# Ubuntu/Debian
sudo apt-get install lsof

# macOS
brew install lsof
```

## Next Steps

1. Start the three terminals as shown above
2. Let the system run through the handshake
3. Analyze the captured logs
4. Compare actual vs expected protocol behavior
5. Fix identified issues
6. Re-test to verify fixes

## Full Documentation

- See [N3FJP_TEST_QUICK_REF.md](N3FJP_TEST_QUICK_REF.md) for quick commands
- See [docs/n3fjp_testing_guide.md](docs/n3fjp_testing_guide.md) for complete guide
- See [docs/n3fjp_protocol_debugging.md](docs/n3fjp_protocol_debugging.md) for advanced usage

Ready to debug! 🚀
