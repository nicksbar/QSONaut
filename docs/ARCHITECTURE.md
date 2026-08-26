# QSONaut Architecture Diagram

## High-Level Architecture

```mermaid
graph TB
    subgraph QSONaut_Client["QSONaut Client v0.2.3"]
        direction TB
        A[apps/qsonaut<br/>Main CLI]
        A --> B[crates/qsonaut-gui<br/>eframe 0.33 + wgpu]
        A --> C[crates/qsonaut-core<br/>Config + Events]
        A --> F[crates/qsonaut-audio<br/>cpal 0.15]
        A --> G[crates/qsonaut-automation<br/>Event system]
        A --> H[crates/qsonaut-server-client<br/>WSS sync]
        A --> I[crates/qsonaut-log<br/>ADIF]
        A --> J[crates/qsonaut-pskreporter<br/>UDP]
        
        B --> F
        G --> H
        G --> I
    end
    
    subgraph QSONaut_Server["QSONaut Server v0.1.0"]
        direction TB
        K[apps/qsonaut-server<br/>Axum 0.8.9]
        K --> L[crates/qsonaut-api<br/>HTTP + WS]
        K --> M[crates/qsonaut-contests<br/>19 templates]
        K --> N[crates/qsonaut-protocol<br/>Wire formats]
        K --> O[crates/qsonaut-store<br/>PostgreSQL]
        K --> P[web<br/>Svelte 5 UI]
    end
    
    subgraph External["External Services"]
        Q[mfsk-core 0.10.0 unreleased<br/>DSP/Decoding]
        U[cw-dit v0.1.0<br/>Morse/CW DSP]
        R[rigwright v0.1.10<br/>Radio HAL and drivers]
        S[PSK Reporter<br/>UDP]
        T[Discord/IRC<br/>Automation]
    end
    
    Q --> B
    U --> B
    F --> Q
    R --> A
    S --> I
    T --> G
    H --> K
    K --> P
```

### Data Flow

```mermaid
sequenceDiagram
    participant User
    participant GUI as qsonaut-gui
    participant Decoder as mfsk-core
    participant Audio as qsonaut-audio
    participant Radio as rigwright
    participant Server as qsonaut-server
    participant DB as PostgreSQL
    
    User->>GUI: Select mode/frequency
    GUI->>Radio: Set tuning (CI-V)
    Radio-->>GUI: ACK
    
    User->>GUI: Enable RX
    GUI->>Audio: Start capture
    Audio->>Decoder: Decimated 12 kHz audio
    Decoder->>Decoder: Decode active WSJT-family mode
    Decoder-->>GUI: Decoded stations
    
    opt Server Connected
        GUI->>Server: Presence publish
        GUI->>Server: QSO logs (idempotent)
        Server-->>GUI: Event catalog
        Server-->>GUI: Shared channels
    end
    
    GUI->>GUI: UI update
    GUI->>DB: (if server) Persist logs
```

---

## Component Dependencies

### QSONaut Client Dependency Graph

```mermaid
graph LR
    qsonaut[apps/qsonaut]
    audio[qsonaut-audio]
    core[qsonaut-core]
    gui[qsonaut-gui]
    log[qsonaut-log]
    automation[qsonaut-automation]
    psk[qsonaut-pskreporter]
    server-client[qsonaut-server-client]
    accelerate[qsonaut-accelerate]
    mfsk[mfsk-core]
    cwdit[cw-dit<br/>cwdit-dsp + cwdit-morse]
    rigwright[rigwright<br/>v0.1.10]
    
    qsonaut -->|path| gui
    qsonaut -->|path| core
    qsonaut -->|path| audio
    qsonaut -->|path| log
    qsonaut -->|path| automation
    qsonaut -->|path| psk
    qsonaut -->|path| server-client
    qsonaut -->|path| accelerate
    qsonaut -->|git| rigwright
    
    gui -->|git| mfsk
    gui -->|path| cwdit
    gui -->|path| audio
    gui -->|path| acceleration
    gui -->|path| automation
    gui -->|path| server-client
    gui -->|path| psk
    
    psk -->|path| core
    server-client -->|path| core
    log -->|path| core
    automation -->|path| core
```

---

## Server API Contract

```mermaid
erDiagram
    DEVICE_AUTH ||--o{ STATION_PRESENCE : "PUT"
    STATION_PRESENCE ||--o{ QSO_LOG : "POST"
    QSO_LOG ||--o{ SERVER_STORE : "persist"
    SERVER_STORE ||--o{ CLIENT_SYNC : "broadcast"
    CLIENT_SYNC ||--o{ SHARED_CHANNELS : "publish/read"
    SHARED_CHANNELS ||--o{ ADMIN_UI : "display"
    ADMIN_UI ||--o{ CLUB_OPS : "manage"
```

### WebSocket Messages (qsonaut.v1)

```mermaid
graph TB
    A[Client] -->|1. presence_publish| B[Server]
    B -->|ack| A
    A -->|2. qso_log| B
    B -->|3. validate_idempotency| A
    B -->|4. store_in_postgres| A
    A -->|5. diagnostic_report| B
    B -->|6. share_with_channel| A
    B -->|7. broadcast_to_clients| A
    A -->|8. shared_message| B
    B -->|9. persist_and_broadcast| A
```

---

## Safety Model

```mermaid
graph TB
    A[Operator Input] --> B{TX Request}
    B -->|Global Disarm| C[PTT Release]
    B -->|Manual Send| D{Safety Check}
    D -->|TX Armed| E[Send PTT]
    D -->|TX Disarmed| F[Reject]
    B -->|Automation| G{Permission Check}
    G -->|Granted| H{TX Armed Check}
    H -->|Armed| E
    H -->|Disarmed| F
    F --> I[Log Rejection]
    E --> I
```

---

## Version Relationships

| Component | Version | Notes |
|-----------|---------|-------|
| QSONaut Client | 0.2.3 | Main feature branch |
| QSONaut Server | 0.1.0 | Independent release |
| mfsk-core | 0.10.0 (unreleased) | Git dependency; pinned upstream commit |
| rigwright | 0.1.10 | crates.io |
| eframe | 0.33 | GUI framework |
| tokio | 1.0 | Async runtime |
| sqlx | 0.8.6 | Server DB |

---

## Recent Changes Summary

### QSONaut v0.2.3 (2026-08-14)
- ✅ Server integration (v0.1.0)
- ✅ Multi-radio selection
- ✅ CI-V scope controls
- ✅ Automation events
- ✅ Persistent UI state

### mfsk-core 0.10.0 (unreleased upstream)
- ✅ FT8 WSJT-X parity
- ✅ WSPR full integration
- ✅ Q65 precision fixes
- ✅ Better test coverage

### QSONaut Server v0.1.0
- ✅ Authenticated WebSocket
- ✅ Contest catalog (19 templates)
- ✅ Club operations
- ✅ Station tokens
