use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    Radio,
    Audio,
    Decoder,
    Transmit,
    Logging,
    Automation,
    Connector,
    AiCapability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    Starting,
    Ready,
    Degraded,
    Disconnected,
    Stopping,
    Stopped,
    Failed,
}

pub fn is_valid_component_transition(
    previous: Option<ComponentState>,
    next: ComponentState,
) -> bool {
    let Some(previous) = previous else {
        return matches!(next, ComponentState::Starting | ComponentState::Stopped);
    };
    if previous == next {
        return true;
    }
    matches!(
        (previous, next),
        (
            ComponentState::Starting,
            ComponentState::Ready
                | ComponentState::Stopping
                | ComponentState::Stopped
                | ComponentState::Failed
        ) | (
            ComponentState::Ready,
            ComponentState::Degraded
                | ComponentState::Disconnected
                | ComponentState::Stopping
                | ComponentState::Failed
        ) | (
            ComponentState::Degraded,
            ComponentState::Ready
                | ComponentState::Disconnected
                | ComponentState::Stopping
                | ComponentState::Failed
        ) | (
            ComponentState::Disconnected,
            ComponentState::Starting | ComponentState::Stopping | ComponentState::Failed
        ) | (
            ComponentState::Stopping,
            ComponentState::Stopped | ComponentState::Failed
        ) | (ComponentState::Stopped, ComponentState::Starting)
            | (
                ComponentState::Failed,
                ComponentState::Starting | ComponentState::Stopping
            )
    )
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    ComponentStateChanged {
        component: Component,
        state: ComponentState,
        detail: String,
    },
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
        #[allow(dead_code)]
        grid: String,
        #[allow(dead_code)]
        state: String,
        #[allow(dead_code)]
        country: String,
        #[allow(dead_code)]
        time_on: String,
        #[allow(dead_code)]
        report_received: String,
        #[allow(dead_code)]
        operation_mode: String,
        #[allow(dead_code)]
        contest_exchange_received: String,
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
    AutomationResult {
        source: String,
        fields: BTreeMap<String, String>,
    },
    CommandResult(crate::CommandResult),
    ShutdownRequested,
}

#[derive(Clone)]
pub struct AppEventBus {
    tx: broadcast::Sender<AppEvent>,
    lifecycle: Arc<Mutex<BTreeMap<Component, ComponentState>>>,
}

impl AppEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            lifecycle: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn publish(&self, event: AppEvent) {
        if let AppEvent::ComponentStateChanged {
            component, state, ..
        } = &event
        {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .expect("event lifecycle lock poisoned");
            if !is_valid_component_transition(lifecycle.get(component).copied(), *state) {
                return;
            }
            lifecycle.insert(*component, *state);
        }
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::{is_valid_component_transition, AppEvent, AppEventBus, Component, ComponentState};

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

    #[test]
    fn lifecycle_transitions_accept_recovery_and_reject_invalid_jumps() {
        assert!(is_valid_component_transition(
            None,
            ComponentState::Starting
        ));
        assert!(is_valid_component_transition(
            Some(ComponentState::Starting),
            ComponentState::Ready
        ));
        assert!(is_valid_component_transition(
            Some(ComponentState::Disconnected),
            ComponentState::Starting
        ));
        assert!(!is_valid_component_transition(
            Some(ComponentState::Ready),
            ComponentState::Starting
        ));
    }

    #[test]
    fn lifecycle_events_are_typed_and_serializable() {
        assert_eq!(
            serde_json::to_string(&Component::Audio).unwrap(),
            "\"audio\""
        );
        assert_eq!(
            serde_json::to_string(&ComponentState::Ready).unwrap(),
            "\"ready\""
        );
    }

    #[test]
    fn event_bus_rejects_invalid_lifecycle_jumps() {
        let bus = AppEventBus::new(4);
        let mut subscriber = bus.subscribe();
        bus.publish(AppEvent::ComponentStateChanged {
            component: Component::Radio,
            state: ComponentState::Starting,
            detail: "starting".to_string(),
        });
        bus.publish(AppEvent::ComponentStateChanged {
            component: Component::Radio,
            state: ComponentState::Stopped,
            detail: "stopped".to_string(),
        });
        bus.publish(AppEvent::ComponentStateChanged {
            component: Component::Radio,
            state: ComponentState::Ready,
            detail: "invalid".to_string(),
        });

        assert!(subscriber.try_recv().is_ok());
        assert!(subscriber.try_recv().is_ok());
        assert!(subscriber.try_recv().is_err());
    }
}
