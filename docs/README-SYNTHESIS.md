# QSONaut Codebase Synthesis Report

**Date:** 2026-08-15
**Workspace:** /home/nick/RigForge

## Executive Summary

QSONaut is a two-repository amateur-radio mission control system:
1. **QSONaut** (client) - Desktop application for radio control and decoding
2. **QSONaut-Server** - Independent coordination service

Both repositories use Rust and share versioning conventions but maintain separate codebases.

---

## 1. QSONaut Client Structure

### Version
- **Current:** v0.2.3
- **Last release:** 2026-08-14

### Main Dependencies

| Crate | Purpose | Version | Notes |
|-------|---------|---------|-------|
| mfsk-core | DSP/decoding | 0.9.1 | Path dependency from sibling repo |
| rigwright | CI-V radio HAL | 0.1.2 | Git dependency (https://github.com/nicksbar/rigwright) |
| eframe 0.33 | GUI framework | 0.33 | With wgpu, wayland, vulkan |
| tokio 1 | Async runtime | 1 | Multi-threaded |
| sqlx 0.8.6 | Server DB (server only) | 0.8.6 | PostgreSQL, migrations |

### Architecture (from apps/qsonaut)
```
apps/qsonaut (main CLI entry)
├── qsonaut-audio (cpal 0.15 - audio I/O and 48→12 kHz decimation)
├── qsonaut-core (config + event bus)
├── qsonaut-gui (eframe GUI with FT8/FT4 mode workflows)
├── qsonaut-log (ADIF import/export)
├── mfsk-core (FT8/FT4/FST4/JT9/JT65/Q65/WSPR/MSK144)
├── qsonaut-pskreporter (UDP reporting)
├── qsonaut-server-client (WSS sync, v0.29 tokio-tungstenite)
├── qsonaut-automation (event-driven components)
└── rigwright (external CI-V radio control)
```

### Key Features (v0.2.3)
- ✅ Optional server integration via WebSocket
- ✅ Multi-radio selection with model-aware profiles
- ✅ Persistent UI state (window geometry, radio selection)
- ✅ CI-V scope controls (IC-7300, FTdx101, FT-857D)
- ✅ Contest workflow with Fox/Hound roles
- ✅ ADIF import/export
- ✅ Achievement Hunter persistence
- ✅ Automation with permission-gated actions
- ✅ Server-sync for presence, logs, diagnostics, shared channels

---

## 2. QSONaut Server Structure

### Version
- **Current:** v0.1.0
- **Last commit:** a4b6a18 (source deployment + container automation)

### Architecture
```
apps/qsonaut-server (deployable Rust process)
├── qsonaut-api (axum 0.8.9 HTTP + WS)
│   └── OpenAPI v1 at /api/v1/openapi.json
├── qsonaut-contests (19 built-in contest templates)
│   └── Synced to PostgreSQL on startup
├── qsonaut-protocol (versioned wire formats)
├── qsonaut-store (sqlx PostgreSQL, migrations)
└── web (Svelte 5 management UI)
    ├── npm ci + npm run dev
    └── Static output served by Rust process
```

### API Endpoints
- `POST /api/v1/auth/device` - Device enrollment (bearer token)
- `PUT /api/v1/stations/presence` - Station presence updates
- `POST /api/v1/logs` - Idempotent QSO submission
- `GET /api/v1/ws` - WebSocket sync (qsonaut.v1 protocol)
- `/api/v1/openapi.json` - OpenAPI spec

### Capabilities (WebSocket)
- `events:read` - Event/catalog snapshots
- `presence:write` - Station presence
- `logs:write` - QSO logs (idempotent via UUID)
- `diagnostics:write` - Diagnostic snapshots
- `messages:read/write` - Shared channel traffic

---

## 3. Relationship Between Client & Server

### Integration Model (docs/qsonaut-client-sync.md)
```
QSONaut Client (v0.2.3+)
    ↓ Optional, opt-in, disabled by default
QSONaut Server (v0.1.0)
    ↓ PostgreSQL persistence
Management UI (Svelte)
```

### Key Integration Points

| Direction | Feature | Default | Notes |
|-----------|---------|---------|-------|
| Client → Server | Station presence | False | Publishes UUID, platform, radio metadata |
| Client → Server | QSO logs | False | Idempotent via UUID, no server commands |
| Server → Client | Event catalog sync | False | Contest definitions, schedules |
| Server → Client | Shared channels | False | Text messages for coordination |
| Client → Server | Diagnostics | False | Manual approval required |

### Authentication
- **Device tokens:** 90-day SHA-256 hashed tokens
- **Enrollment:** `POST /api/v1/auth/device` with callsign/password
- **Browser tokens:** Separate from native client (Station link in UI)
- **Revocation:** Password reset or explicit token deletion

---

## 4. Recent Changes (Last 6 Months)

### QSONaut Client (v0.2.3)
**2026-08-14:**
- Added optional QSONaut Server integration
- Server sync for events, presence, logs, diagnostics
- Automation events for server sync/publish

**2026-08-13:**
- Model-aware radio selection (IC-7300 validated)
- Side-by-side waterfall monitoring
- IC-7300 scope controls (sweep speed, VBW, color palette)
- Live CI-V diagnostics

**2026-08-12:**
- v0.2.0: Contest profiles, Fox/Hound roles, Achievement Hunter
- v0.2.1: Windows icon build fix

### mfsk-core (v0.9.1)
**Recent fixes:**
- FT8: Restored WSJT-X coarse-sync lag bounds (±2.5s)
- Q65: Restored ±1.0s Δt window
- WSPR: Full wsprd integration (4-pass, Fano metric)
- Better test coverage and precision guards

### QSONaut Server
- Initial foundation
- Authenticated WebSocket sync
- Shared automation channels
- Club operations/governance
- Contest catalog (19 templates)

---

## 5. README Update Needs

### QSONaut/README.md
**Current issues:**
1. ✅ Mentions mfsk-core v0.9.1 correctly
2. ✅ Describes server integration exists
3. ❌ **Needs:** Explicit version for QSONaut Server
4. ❌ **Needs:** Mention automation features
5. ❌ **Needs:** Clarify server integration is opt-in, disabled by default
6. ❌ **Needs:** Update rigwright dependency info

### QSONaut-Server/README.md
**Current issues:**
1. ❌ **Needs:** Version bump (currently v0.1.0)
2. ❌ **Needs:** Verify contest catalog count (docs say 19)
3. ❌ **Needs:** Update development setup instructions
4. ❌ **Needs:** Mention Yahaml independence

### Cross-Repository Notes
- Both repos use workspace versioning (v0.2.3 vs v0.1.0)
- Client pins rigwright to GitHub tag v0.1.2
- Server has no dependency on client code
- Client uses path dependency on mfsk-core from sibling repo

---

## 6. Build Requirements

### QSONaut Client
```bash
# Linux/WSL
sudo apt-get install libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev

# Optional GPU rendering
GALLIUM_DRIVER=d3d12 MESA_D3D12_DEFAULT_ADAPTER_NAME=AMD \
cargo run --release -p qsonaut -- --gui

# Windows
cargo run --release -p qsonaut -- --gui

# ARM64
cargo run --release -p qsonaut -- --gui
```

### QSONaut Server
```bash
# Docker Compose
docker compose -f deploy/compose.yaml --env-file .env up --build -d

# Native
npm --prefix web ci && npm --prefix web run build
cargo build --release -p qsonaut-server
```

---

## 7. Safety Model

**Critical:** QSONaut is pre-alpha, not validated for unattended operation.

- TX/PTT control always requires operator confirmation
- Global "DISARM ALL TX" control is mandatory
- Automation transmit is disabled by default
- Server commands imply no transmit authority

---

## 8. Next Steps for README Updates

1. **QSONaut/README.md:**
   - Add QSONaut Server version
   - Mention automation features prominently
   - Clarify server integration default state
   - Update rigwright dependency note

2. **QSONaut-Server/README.md:**
   - Update version to v0.1.0
   - List all 19 contest templates
   - Add recent feature highlights
   - Verify Docker setup steps

3. **Both:**
   - Add versioning policy reference (docs/versioning-and-releases.md)
   - Link to integration docs (docs/qsonaut-client-sync.md)
   - Update safety disclaimers
