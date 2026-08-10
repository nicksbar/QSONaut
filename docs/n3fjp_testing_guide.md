# N3FJP Protocol Testing Guide

## Quick Start (Copy-Paste Ready)

### Setup in 3 Terminal Windows

**Terminal 1 - Start N3FJP Server:**
```bash
cd /home/nick/YAHAML
npm run dev:api
```

**Terminal 2 - Start MITM Relay:**
```bash
cd /home/nick/YAHAML
npm run relay:default
# Or: npm run relay -- 2000 localhost 1000
```

**Terminal 3 - Start UI & API:**
```bash
cd /home/nick/YAHAML
npm run dev:all
# Then configure client to connect to localhost:2000 instead of localhost:1000
```

## Testing Workflow

### Step 1: Check Port Availability
```bash
./scripts/n3fjp_test_setup.sh check
```

This shows:
- ✓ Client port 2000 available/in use
- ✓ Server port 1000 available/in use
- ✓ Recent captured logs

### Step 2: Start Server
```bash
./scripts/n3fjp_test_setup.sh server
```

Expected output:
```
[INFO] Starting API server for relay testing...
[Server] Listening on port 3000
```

### Step 3: Start Relay
```bash
./scripts/n3fjp_test_setup.sh relay
```

Expected output:
```
[INFO] Starting MITM relay...
  Client connects to:    localhost:2000
  Relay forwards to:     localhost:1000
  Logs saved to:         /home/nick/YAHAML/captures/
[Relay] Listening for clients on port 2000...
[Relay] Forwarding to localhost:1000...
```

### Step 4: Run Client
```bash
cd /home/nick/YAHAML
npm run dev:all
```

In the UI:
1. Configure client to connect to `localhost:2000` (the relay)
2. Click "Connect" or start the logging session
3. Interact with the server normally

### Step 5: Analyze Results
```bash
./scripts/n3fjp_test_setup.sh analyze
```

This shows:
- All client commands sent
- All server responses received
- Exact hex and ASCII representations
- Connection timeline

## Understanding Captured Data

### Raw Log Format (`captures/n3fjp_mitm_*.log`)
```
[2026-02-01T15:30:45.123Z] [conn-1] Client connected from 127.0.0.1:54321
[2026-02-01T15:30:45.234Z] [conn-1] Connected to server at localhost:1000
[2026-02-01T15:30:45.345Z] [conn-1] client→server (25 bytes): GETCONTESTNUMBER\r\n
[2026-02-01T15:30:45.456Z] [conn-1] server→client (50 bytes): CONTESTNUMBER,1,2024\r\n
[2026-02-01T15:30:45.567Z] [conn-1] Socket closed by client
```

### Structured Data (`captures/n3fjp_mitm_*.json`)
```json
{
  "relay_config": {
    "client_port": 2000,
    "server_host": "localhost",
    "server_port": 1000,
    "start_time": "2026-02-01T15:30:45.123Z"
  },
  "connections": [
    {
      "id": "conn-1",
      "client_address": "127.0.0.1",
      "client_port": 54321,
      "server_address": "localhost",
      "server_port": 1000,
      "start_time": "2026-02-01T15:30:45.123Z",
      "messages": [
        {
          "direction": "client→server",
          "timestamp": "2026-02-01T15:30:45.345Z",
          "raw_bytes": "474554434f4e5445535450524f4252414e555a0d0a",
          "ascii": "GETCONTESTNUMBER\r\n",
          "length": 25
        }
      ]
    }
  ]
}
```

## Analyzing Protocol Issues

### Issue: "Client stays on 'Attempting to Connect'"

**Check List:**

1. **Server running?**
   ```bash
   lsof -i :1000
   ```
   Should show the API server listening.

2. **Relay running?**
   ```bash
   lsof -i :2000
   ```
   Should show relay listening.

3. **Client connecting to relay?**
   Check relay logs:
   ```bash
   tail -f captures/n3fjp_mitm_*.log | grep "Client connected"
   ```

4. **Any data being sent?**
   ```bash
   grep "client→server" captures/n3fjp_mitm_*.log
   ```
   If nothing here, client is blocking before sending.

5. **Server responding?**
   ```bash
   grep "server→client" captures/n3fjp_mitm_*.log
   ```
   If nothing here, server isn't responding.

### Issue: "Protocol Mismatch"

Compare actual vs expected:
```bash
# Show all client commands
grep "client→server" captures/n3fjp_mitm_*.log | head -5

# Show all server responses
grep "server→client" captures/n3fjp_mitm_*.log | head -5
```

If format is wrong (binary vs text, wrong delimiters), check:
- Message encoding
- Line terminators (should be `\r\n`)
- Command formatting

### Issue: "Server crashes or disconnects"

Look for error messages or EOF:
```bash
grep -E "error|closed|EOF" captures/n3fjp_mitm_*.log
```

## Protocol Specification Reference

Expected N3FJP handshake sequence:

```
CLIENT → SERVER: GETCONTESTNUMBER\r\n
SERVER → CLIENT: CONTESTNUMBER,<num>,<year>\r\n

CLIENT → SERVER: GETSERVERVERSION\r\n
SERVER → CLIENT: SERVERVERSION,<version>\r\n

CLIENT → SERVER: GETOPERATORCALL\r\n
SERVER → CLIENT: OPERATORCALL,<callsign>\r\n
```

Each message should be:
- Text-based (ASCII/UTF-8)
- Ended with `\r\n` (carriage return + line feed)
- Comma-separated values for responses
- Case-sensitive commands

## Troubleshooting

### "npm: command not found"
```bash
# Install Node.js and npm
curl -fsSL https://deb.nodesource.com/setup_18.x | sudo -E bash -
sudo apt-get install -y nodejs

# Or use nvm
curl https://raw.githubusercontent.com/creationix/nvm/master/install.sh | bash
nvm install 18
nvm use 18
```

### "Port already in use"
```bash
# Find what's using port 2000
lsof -i :2000

# Kill the process (if safe)
kill -9 <PID>

# Or use different port
npm run relay -- 3000 localhost 1000
```

### "TypeScript compilation errors"
```bash
# Rebuild TypeScript
npm run build

# Clear TypeScript cache
rm -rf dist/
npm run build
```

### "No captured logs"
1. Check that relay is actually running
2. Check that client connected to relay (port 2000), not server (port 1000)
3. Check captures directory exists: `ls -la captures/`
4. Check file permissions: `chmod 777 captures/`

## Performance Monitoring

Monitor relay performance while running:
```bash
# Real-time log monitor
./scripts/n3fjp_test_setup.sh monitor

# Message count by direction
grep -c "client→server" captures/n3fjp_mitm_*.log
grep -c "server→client" captures/n3fjp_mitm_*.log

# Data volume
du -h captures/n3fjp_mitm_*
```

## Next Steps After Analysis

1. **Document findings** in a new MITM capture summary
2. **Update protocol docs** if behavior differs from spec
3. **Fix identified issues** in client or server code
4. **Re-test** to verify fixes
5. **Keep logs** as reference for future debugging

## Advanced Usage

### Change Network Ports
```bash
# Relay on 3000, forward to 192.168.1.50:1000
./scripts/n3fjp_test_setup.sh relay \
  --client-port 3000 \
  --server-host 192.168.1.50 \
  --server-port 1000
```

### Test Multiple Clients
```bash
# Terminal 1: Server
./scripts/n3fjp_test_setup.sh server

# Terminal 2: Relay
./scripts/n3fjp_test_setup.sh relay

# Terminal 3: First client
npm run dev:all -- --port 3001

# Terminal 4: Second client
npm run dev:all -- --port 3002

# Analyze - both clients will be in one capture
./scripts/n3fjp_test_setup.sh analyze
```

### Parse Hex Manually
```bash
# Hex to ASCII
echo "474554434f4e5445535450524f4252414e555a0d0a" | xxd -r -p

# ASCII to Hex
echo "GETCONTESTNUMBER" | xxd -p
```

## Files Reference

| File | Purpose |
|------|---------|
| `scripts/n3fjp_mitm_relay.ts` | MITM relay implementation |
| `scripts/analyze_n3fjp_log.ts` | Log analyzer tool |
| `scripts/n3fjp_test_setup.sh` | Test setup orchestration |
| `npm run dev:api` | Test API server for relay scenarios |
| `captures/` | Captured logs directory |
| `docs/n3fjp_protocol_debugging.md` | Quick reference guide |

## Summary

This testing infrastructure lets you:
1. **Capture** all bidirectional protocol communication
2. **Analyze** exact message formats and timing
3. **Debug** connection issues at the protocol level
4. **Verify** client/server implementation correctness
5. **Document** protocol behavior for future reference
