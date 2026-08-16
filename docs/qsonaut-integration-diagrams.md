# QSONaut Server Integration - Dependencies Map

## Overview Diagram

```mermaid
graph TB
    subgraph "QSONaut Desktop Client"
        A[QSONaut Main App<br/>apps/qsonaut]
        B[GUI Layer<br/>crates/qsonaut-gui]
        C[Automation Layer<br/>crates/qsonaut-automation]
        D[Server Client<br/>crates/qsonaut-server-client]
        
        A --> B
        B --> C
        C -.->|ServerMessage| D
        B -.->|Presence, Logs, Diagnostics| D
    end
    
    subgraph "QSONaut Server"
        E[REST API<br/>crates/qsonaut-api]
        F[WebSocket Handler<br/>crates/qsonaut-api/realtime.rs]
        G[Database<br/>crates/qsonaut-store]
        H[Contest Catalog<br/>crates/qsonaut-contests]
        I[Management UI<br/>web/]
        
        E --> F
        E --> G
        F -.->|WebSocket| D
        E --> I
        H -.->|Sync to DB| G
    end
    
    subgraph "External"
        J[Browser Management<br/>UI at port 8080]
        K[PostgreSQL<br/>Database]
        
        J -.->|POST /api/v1/auth/device/session| E
        K <--> G
    end
    
    D -.->|Authorization: Bearer| J
    J -.->|Device Token| E
```

## Data Flow Diagrams

### 1. Device Token Flow

```mermaid
sequenceDiagram
    participant UI as QSONaut Server UI
    participant API as REST API
    participant DB as PostgreSQL
    participant Client as QSONaut Desktop
    
    UI->>API: POST /api/v1/auth/device/session
    API->>DB: Query existing devices
    DB-->>API: Return device tokens
    API-->>UI: Display token (once only)
    UI->>Client: User pastes token into config
    Client->>Client: Store token in qsonaut.toml
```

### 2. WebSocket Connection Flow

```mermaid
sequenceDiagram
    participant Client as QSONaut Desktop
    participant WS as WebSocket Connection
    participant API as QSONaut Server
    
    Client->>WS: WSS://server/api/v1/ws
    WS->>API: WebSocket upgrade
    API->>WS: Accept + qsonaut.v1 protocol
    WS->>Client: Send Welcome
    Client->>WS: Hello {client_version}
    WS->>WS: Sync request
    Client->>WS: Presence {station_info}
    Client->>WS: QSO log {contact}
    Client->>WS: Diagnostic {status}
    Client->>WS: Channel message {text}
    Client->>WS: Ping (heartbeat)
    
    WS-->>Client: Welcome user
    WS-->>Client: Presence accepted
    WS-->>Client: Log accepted
    WS-->>Client: Channel accepted
    WS-->>Client: Pong
```

### 3. Station Presence Flow

```mermaid
sequenceDiagram
    participant Client as QSONaut Desktop
    participant WS as WebSocket
    participant API as QSONaut Server
    
    Client->>WS: PUT /api/v1/stations/presence
    JSON payload:
      instance_id: uuid
      callsign: N1CALL
      radio_model: IC-7300
      frequency: 14.085 MHz
      status: online
      
    WS-->>Client: Presence accepted (acknowledgment)
```

### 4. QSO Logging Flow

```mermaid
sequenceDiagram
    participant Client as QSONaut Desktop
    participant WS as WebSocket
    participant API as QSONaut Server
    
    Client->>WS: POST /api/v1/logs
    JSON payload:
      operator_callsign: N1CALL
      callsign: N2TEST
      mode: FT8
      frequency: 14.085 MHz
      points: 126
      timestamp: ISO8601
      idempotency_key: uuid
      
    WS-->>Client: Log accepted (duplicate check)
```

### 5. Event Catalog Sync Flow

```mermaid
sequenceDiagram
    participant Client as QSONaut Desktop
    participant WS as WebSocket
    participant Server as QSONaut Server
    
    Server->>WS: Snapshot {
      events: [active_event],
      contest_templates: [...],
      channel_messages: [...]
    }
    
    WS-->>Client: Receive snapshot
    Client->>Client: Update UI with events
    Client->>Client: Update contest options
```

## API Endpoints Reference

### Authentication

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| POST | `/api/v1/auth/device` | None | Register device, get bearer token |
| POST | `/api/v1/auth/device/session` | Login required | Browser session token |
| DELETE | `/api/v1/auth/device/{id}` | Device token | Revoke device token |

### Presence

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| PUT | `/api/v1/stations/presence` | Device token | Publish station presence |

### QSO Logging

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| POST | `/api/v1/logs` | Device token | Submit QSO log |
| GET | `/api/v1/logs` | Device token | Query logs |

### Diagnostics

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| GET | `/api/v1/diagnostics` | Device token | Get diagnostic reports |
| POST | `/api/v1/diagnostics` | Device token | Send diagnostic report |

### Channels

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| POST | `/api/v1/channel-messages` | Device token | Publish message |
| GET | `/api/v1/channel-messages` | Device token | List messages |

### Events

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| GET | `/api/v1/events` | Device token | List events |
| POST | `/api/v1/events` | Device token | Create event |
| PATCH | `/api/v1/events/{id}/status` | Device token | Update event status |

### Contest Templates

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| GET | `/api/v1/contest-templates` | None | Get all templates |

### WebSocket

| Endpoint | Protocol | Purpose |
|----------|----------|---------|
| GET `/api/v1/ws` | qsonaut.v1 | Real-time sync |

## Component Dependencies

### QSONaut Client Dependencies

```mermaid
graph LR
    A[QSONaut Main] --> B[qsonaut-gui]
    B --> C[qsonaut-server-client]
    C --> D[tokio-tungstenite]
    C --> E[serde/serde_json]
    C --> F[uuid]
    C --> G[anyhow]
    
    B --> H[qsonaut-automation]
    H -->|ServerMessage| C
```

### QSONaut Server Dependencies

```mermaid
graph LR
    A[qsonaut-api] --> B[axum]
    A --> C[qsonaut-protocol]
    A --> D[qsonaut-store]
    A --> E[utoipa]
    
    D --> F[postgres]
    C --> G[serde]
    A --> H[WebSocket handler]
    H -->|WebSocket| I[qsonaut-server-client]
```

## Version Synchronization Points

| Point | Location | Version | Notes |
|-------|----------|---------|-------|
| Cargo workspace | QSONaut/Cargo.toml | 0.2.3 | Main client version |
| Protocol version | qsonaut-protocol/src/lib.rs | v1 | API version string |
| WebSocket protocol | qsonaut-server-client/src/lib.rs | qsonaut.v1 | Subprotocol name |
| Message format | qsonaut-protocol/src/lib.rs | Structured | JSON envelope |

## Security Boundaries

```mermaid
graph TB
    subgraph "Client-Side (QSONaut)"
        A[Operator credentials<br/>callsign/password]
        B[Audio input/output]
        C[Radio serial device]
        D[Local audio samples]
        E[QSO log files]
    end
    
    subgraph "Server-Side (QSONaut Server)"
        F[Device token<br/>SHA-256 hashed]
        G[Operator profile<br/>callsign/display_name]
        H[PostgreSQL database]
        I[Station presence]
        J[QSO logs]
        K[Channel messages]
    end
    
    A -->|HTTPS/SSO| F
    F -.->|Authorization| H
    B -->|Raw audio| D
    C -->|CI-V| E
    D -->|Optional| J
    E -->|Optional| J
```

## Data Retention Policy

| Data Type | Retention | Scope |
|-----------|-----------|-------|
| Device tokens | 90 days | Server |
| Station presence | Server-configured | Server |
| QSO logs | Server-configured | Server |
| Channel messages | Server-configured | Server |
| Diagnostics | Server-configured | Server |
| Contest templates | Permanent | Server |

## Future Enhancements

1. **N3FJP Integration**
   - Club roster synchronization
   - Governance operations
   - Election management

2. **Real-time Features**
   - Live event updates
   - Multi-operator sessions
   - Automated contest reporting

3. **Extended Diagnostics**
   - Hardware health checks
   - Performance metrics
   - Resource utilization
