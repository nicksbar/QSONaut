use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum AppEvent {
    DeviceDiscovered {
        subsystem: String,
        name: String,
        detail: String,
    },
    DeviceDisconnected {
        subsystem: String,
        name: String,
    },
    ShutdownRequested,
}

#[derive(Clone)]
pub struct AppEventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl AppEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, event: AppEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }
}
