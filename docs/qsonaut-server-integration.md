# QSONaut Server Integration

## Overview

QSONaut Server is the independent coordination service for QSONaut operators, clubs, and group events. QSONaut (the desktop client) optionally connects to QSONaut Server for event/catalog synchronization, presence tracking, QSO logging, and shared channel messaging.

This document describes the integration architecture, dependencies, and data flows.

## Architecture

### Components

| Component | Location | Purpose |
|-----------|----------|---------|
| QSONaut Desktop Client | `QSONaut/` | Native GUI application for radio control and digital modes |
| QSONaut Server | `QSONaut-Server/` | REST API and WebSocket server for coordination |
| qsonaut-server-client | `QSONaut/crates/qsonaut-server-client/` | WebSocket client library |

### Version Synchronization

Both repositories use semantic versioning and are synchronized through:

```toml
# QSONaut/Cargo.toml
[workspace.package]
version = "0.2.3"  # Must match QSONaut-Server protocol versions
```

## Connection Flow

```mermaid
sequenceDiagram
    participant Client as QSONaut Desktop
    participant ClientAPI as qsonaut-server-client
    participant WS as WebSocket
    participant Server as QSONaut Server
    participant Auth as Browser/Management UI

    Client->>Auth: Open Station Link in Management UI
    Auth->>Server: POST /api/v1/auth/device/session
    Server->>Auth: Display device token (once)
    Auth->>Client: User pastes token into config
    Client->>ClientAPI: spawn() with config
    ClientAPI->>WS: Connect to /api/v1/ws
    WS->>Server: WebSocket handshake (qsonaut.v1 protocol)
    Server->>WS: Send Welcome + Snapshot
    WS->>Client: Receive event/catalog snapshot
    alt Server Commands
        Client->>ClientAPI: publish_presence()
        Client->>WS: Presence message
        Client->>ClientAPI: publish_log()
        Client->>WS: QSO log
        Client->>ClientAPI: publish_diagnostic()
        Client->>WS: Diagnostic report
        Client->>ClientAPI: publish_channel_message()
        Client->>WS: Channel message
    end
    WS->>Client: Server acknowledgements
```

## Data Contracts

### WebSocket Protocol

**Protocol Version:** `qsonaut.v1`

**Authentication:** `Authorization: Bearer <device_token>`

**Message Format:** Versioned JSON envelope

```json
{
  "protocol_version": "v1",
  "event_id": "uuid-v4",
  "message": {
    "type": "presence|log|diagnostic|channel_message|ping",
    "payload": {...}
  }
}
```

### Server Commands (Client → Server)

| Command | Method | Description |
|---------|--------|-------------|
| `Presence` | `PUT /api/v1/stations/presence` | Update station presence |
| `Log` | `POST /api/v1/logs` | Submit QSO log |
| `Diagnostic` | `GET /api/v1/diagnostics` | Send diagnostic report |
| `ChannelMessage` | `POST /api/v1/channel-messages` | Publish to shared channel |
| `Ping` | Heartbeat | Keep connection alive |

### Server Responses (Server → Client)

| Message | Description |
|---------|-------------|
| `Welcome` | Authentication success, user details |
| `Snapshot` | Event catalog, contest templates, channel messages |
| `Error` | Server error notification |
| `ChannelMessagePublished` | Incoming channel message |
| `ChannelMessageAccepted` | Message acknowledged |
| `PresenceAccepted` | Presence acknowledged |
| `LogAccepted` | QSO log acknowledged |
| `DiagnosticAccepted` | Diagnostic report acknowledged |
| `Ack` / `Pong` | Connection heartbeat |

## API Endpoints

### Authentication

- `POST /api/v1/auth/device` - Register device and receive bearer token
- `POST /api/v1/auth/device/session` - Browser session token (for management UI)
- `DELETE /api/v1/auth/device/{token_id}` - Revoke device token

### Presence

- `PUT /api/v1/stations/presence` - Publish station presence

### QSO Logging

- `POST /api/v1/logs` - Submit QSO log (idempotent by key)

### Diagnostics

- `GET /api/v1/diagnostics` - Retrieve diagnostic reports

### Shared Channels

- `POST /api/v1/channel-messages` - Publish message to channel
- `GET /api/v1/channel-messages` - List channel messages

### Events and Catalog

- `GET /api/v1/events` - List events
- `GET /api/v1/contest-templates` - Get contest definitions

### WebSocket

- `GET /api/v1/ws` - WebSocket connection for real-time sync

## QSONaut Server Crate Structure

```
QSONaut-Server/
├── crates/
│   ├── qsonaut-api/          # HTTP API + OpenAPI spec
│   │   ├── auth.rs           # Authentication endpoints
│   │   ├── management.rs     # Club/member/contest management
│   │   ├── realtime.rs       # WebSocket sync
│   │   └── error.rs          # Error handling
│   ├── qsonaut-protocol/     # Shared wire formats (v1)
│   │   ├── lib.rs            # All protocol structs
│   ├── qsonaut-store/        # PostgreSQL persistence
│   │   └── migrations/       # Database schema
│   └── qsonaut-contests/     # Built-in contest catalog
├── apps/
│   └── qsonaut-server        # Deployable server process
└── docs/
    ├── qsonaut-client-sync.md # Sync boundary documentation
    └── scope.md              # Service capabilities
```

## QSONaut Client Crate Structure

```
QSONaut/
├── crates/
│   └── qsonaut-server-client/  # WebSocket client library
│       └── lib.rs              # Connection management
├── crates/
│   ├── qsonaut-gui/            # Desktop GUI (uses ServerClient)
│   └── qsonaut-automation/     # Event-driven automation
│       └── lib.rs              # ServerMessage event type
```

## Server Client Usage (qsonaut-gui)

```rust
// In qsonaut-gui/src/lib.rs

// 1. Create connection config from TOML config
let server_client = (config.server.enabled && 
    !config.server.url.is_empty() && 
    !config.server.device_token.is_empty())
.then(|| {
    ServerClient::spawn(ServerConnectionConfig {
        server_url: config.server.url,
        device_token: config.server.device_token,
        client_version: env!("CARGO_PKG_VERSION"),
    })
});

// 2. Publish presence updates
server_client.as_ref().map(|sc| sc.publish_presence(presence));

// 3. Submit QSO logs
server_client.as_ref().map(|sc| sc.publish_log(log));

// 4. Send diagnostics
server_client.as_ref().map(|sc| sc.publish_diagnostic(diagnostic));

// 5. Publish channel messages
server_client.as_ref().map(|sc| sc.publish_channel_message(channel, message));
```

## TOML Configuration

```toml
[server]
enabled = false
url = "https://radio.example.net"
device_token = ""  # Secret issued by POST /api/v1/auth/device
share_presence = false
share_logs = false
share_radio_details = false
share_diagnostics = false
share_debug_logs = false
```

## Recent Changes

### QSONaut (v0.2.2 → v0.2.3)

| Commit | Description |
|--------|-------------|
| `0fb8e40` | Add optional QSONaut Server synchronization |
| `bfd38b6` | Connect automation rules to server channels |
| `1a37cf3` | Add server controls and diagnostic snapshots |

### QSONaut Server

| Commit | Description |
|--------|-------------|
| `856a334` | Add authenticated QSONaut WebSocket sync |
| `16f285d` | Add shared automation channel transport |
| `559a9e9` | Manage station tokens and diagnostics |

## Dependencies

### QSONaut → QSONaut Server

| Dependency | Version | Purpose |
|------------|---------|---------|
| `qsonaut-server-client` | `0.2.3` (workspace) | WebSocket client |
| `tokio-tungstenite` | `0.29` | WebSocket implementation |
| `serde/serde_json` | `1` | JSON serialization |
| `uuid` | `1` | Event ID generation |
| `anyhow` | `1` | Error handling |

### QSONaut Server

| Dependency | Version | Purpose |
|------------|---------|---------|
| `axum` | - | HTTP API server |
| `postgres` | - | Database persistence |
| `utoipa` | - | OpenAPI generation |

## Security Considerations

1. **Device Tokens**: Bearer tokens are stored as SHA-256 hashes by server
2. **Password Resets**: Revoke all browser sessions and device tokens
3. **Token Revocation**: Clients can revoke current token via `DELETE /api/v1/auth/device`
4. **Browser Isolation**: Session cookies never copied to native client
5. **Optional Integration**: Server connection is completely opt-in

## Future Enhancements

- [ ] N3FJP integration (club roster synchronization)
- [ ] Live contest event synchronization
- [ ] Real-time shared channel broadcasting
- [ ] Automated contest reporting
- [ ] Multi-operator session management

## Testing

### Local Development

```bash
# Start QSONaut Server
cd QSONaut-Server
docker compose -f deploy/postgres-dev.compose.yaml up -d
export QSONAUT_DATABASE_URL=postgresql://qsonaut:password@127.0.0.1:5432/qsonaut
cargo run -p qsonaut-server

# Connect QSONaut Desktop
cd QSONaut
cargo run --release -p qsonaut -- --gui
```

### Token Registration

1. Open QSONaut Server Management UI
2. Navigate to "Station link"
3. Create device name and copy token
4. Paste token into QSONaut config: `device_token = "<token>"`
