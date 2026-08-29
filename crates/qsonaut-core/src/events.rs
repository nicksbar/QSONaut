use std::collections::BTreeMap;
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
    ContestProfileChanged {
        enabled: bool,
        operating_mode: String,
        split_policy: String,
        fox_hound_role: String,
    },
    CallsignHit {
        mode: String,
        call: String,
        snr_db: f32,
        freq_hz: u32,
        message: String,
        directed_to_me: bool,
    },
    QsoLogged {
        mode: String,
        call: String,
        band: String,
        frequency_hz: u64,
    },
    ExternalMessageReceived {
        source: String,
        author: String,
        message: String,
        #[allow(dead_code)]
        channel: String,
    },
    ServerMessageReceived {
        kind: String,
        fields: BTreeMap<String, String>,
    },
    AutomationHook {
        kind: String,
        source: String,
        detail: String,
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

#[cfg(test)]
mod tests {
    use super::{AppEvent, AppEventBus};

    #[test]
    fn publishes_events_to_subscribers() {
        let bus = AppEventBus::new(4);
        let mut subscriber = bus.subscribe();

        bus.publish(AppEvent::ShutdownRequested);

        assert!(matches!(
            subscriber.try_recv(),
            Ok(AppEvent::ShutdownRequested)
        ));
    }

    #[test]
    fn subscribers_are_independent_and_receive_later_events() {
        let bus = AppEventBus::new(4);
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();

        bus.publish(AppEvent::AutomationHook {
            kind: "timer".to_string(),
            source: "test".to_string(),
            detail: "coverage".to_string(),
        });

        assert!(matches!(
            first.try_recv(),
            Ok(AppEvent::AutomationHook { .. })
        ));
        assert!(matches!(
            second.try_recv(),
            Ok(AppEvent::AutomationHook { .. })
        ));
        assert!(first.try_recv().is_err());
        assert!(second.try_recv().is_err());
    }
}
