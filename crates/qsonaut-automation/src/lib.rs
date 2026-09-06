//! Event-driven, permission-gated automation for QSONaut.
//!
//! The design deliberately separates observing events from performing actions.
//! A component can request powerful capabilities in its manifest, but the host
//! must grant them independently before an action is released to the app.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Decode,
    CallsignHit,
    QsoLogged,
    RadioState,
    ContestState,
    OperatorProfile,
    Command,
    ExternalMessage,
    ServerMessage,
    Timer,
    ControlRead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationEvent {
    pub kind: EventKind,
    pub source: String,
    #[serde(default)]
    pub timestamp_ms: u64,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub tags: BTreeSet<String>,
}

impl AutomationEvent {
    pub fn new(kind: EventKind, source: impl Into<String>) -> Self {
        Self {
            kind,
            source: source.into(),
            timestamp_ms: 0,
            fields: BTreeMap::new(),
            tags: BTreeSet::new(),
        }
    }

    pub fn field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.insert(tag.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    UiNotification,
    ExternalSend,
    ServerRead,
    ServerPublish,
    SetCompose,
    RadioControl,
    Transmit,
}

/// Values accepted by the protocol-neutral control automation surface.
///
/// The automation crate deliberately does not depend on Rigwright. The GUI
/// adapter resolves `control` names against the selected HAL profile and
/// rejects values the driver cannot represent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ControlValue {
    Bool(bool),
    U8(u8),
    I32(i32),
    U64(u64),
    Text(String),
    RawHex(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlStep {
    pub control: String,
    pub value: ControlValue,
    #[serde(default)]
    pub wait_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilitySet(pub BTreeSet<Capability>);

impl CapabilitySet {
    pub fn new(capabilities: impl IntoIterator<Item = Capability>) -> Self {
        Self(capabilities.into_iter().collect())
    }

    pub fn contains(&self, capability: Capability) -> bool {
        self.0.contains(&capability)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentManifest {
    pub id: String,
    pub name: String,
    #[serde(default = "default_component_version")]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub subscriptions: BTreeSet<EventKind>,
    #[serde(default)]
    pub requests: CapabilitySet,
}

fn default_component_version() -> String {
    "0.1.0".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExternalSourceConfig {
    Discord {
        /// Environment variable containing the token. Raw secrets do not belong
        /// in component files.
        token_env: String,
        #[serde(default)]
        guild_id: Option<String>,
        channel_ids: Vec<String>,
    },
    Irc {
        server: String,
        #[serde(default = "default_irc_port")]
        port: u16,
        #[serde(default = "default_true")]
        tls: bool,
        nickname: String,
        channels: Vec<String>,
        #[serde(default)]
        password_env: Option<String>,
    },
}

fn default_irc_port() -> u16 {
    6697
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSourceDescriptor {
    pub id: String,
    pub display_name: String,
    pub connected: bool,
}

/// Adapter boundary for Discord, IRC, local sockets, or future sources.
/// Network implementations live outside this crate.
pub trait ExternalSource: Send {
    fn descriptor(&self) -> ExternalSourceDescriptor;
    fn poll(&mut self) -> Result<Vec<AutomationEvent>, ComponentError>;
    fn send(&mut self, target: &str, message: &str) -> Result<(), ComponentError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Notify {
        title: String,
        body: String,
        accent: Option<String>,
    },
    SendExternal {
        source: String,
        target: String,
        message: String,
    },
    ServerSync,
    ServerSendMessage {
        channel: String,
        message: String,
    },
    SetCompose {
        mode: String,
        message: String,
    },
    RadioCommand {
        command: String,
        value: String,
    },
    /// Read one HAL control. The adapter publishes the result as an
    /// automation event using `result_key` for later script references.
    ReadControl {
        control: String,
        result_key: String,
    },
    /// Write one HAL control using the selected driver's native conversion.
    WriteControl {
        control: String,
        value: ControlValue,
    },
    /// Execute a bounded sequence of generic HAL control writes.
    ControlSequence {
        name: String,
        steps: Vec<ControlStep>,
    },
    RequestTransmit {
        mode: String,
        message: String,
    },
}

impl Action {
    pub fn required_capability(&self) -> Capability {
        match self {
            Self::Notify { .. } => Capability::UiNotification,
            Self::SendExternal { .. } => Capability::ExternalSend,
            Self::ServerSync => Capability::ServerRead,
            Self::ServerSendMessage { .. } => Capability::ServerPublish,
            Self::SetCompose { .. } => Capability::SetCompose,
            Self::RadioCommand { .. } => Capability::RadioControl,
            Self::ReadControl { .. } | Self::WriteControl { .. } | Self::ControlSequence { .. } => {
                Capability::RadioControl
            }
            Self::RequestTransmit { .. } => Capability::Transmit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleComponentConfig {
    pub component: ComponentManifest,
    #[serde(default)]
    pub sources: Vec<ExternalSourceConfig>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl RuleComponentConfig {
    pub fn from_toml(source: &str) -> Result<Self, ComponentError> {
        toml::from_str(source).map_err(ComponentError::Config)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub on: EventKind,
    #[serde(default)]
    pub when: Vec<Predicate>,
    pub actions: Vec<ActionTemplate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "match", rename_all = "snake_case")]
pub enum Predicate {
    FieldEquals { field: String, value: String },
    FieldNotEquals { field: String, value: String },
    FieldContains { field: String, value: String },
    FieldLessThan { field: String, value: String },
    FieldGreaterThan { field: String, value: String },
    HasTag { tag: String },
}

impl Predicate {
    pub fn matches(&self, event: &AutomationEvent) -> bool {
        match self {
            Self::FieldEquals { field, value } => event.fields.get(field) == Some(value),
            Self::FieldNotEquals { field, value } => event
                .fields
                .get(field)
                .is_some_and(|candidate| candidate != value),
            Self::FieldContains { field, value } => event
                .fields
                .get(field)
                .is_some_and(|candidate| candidate.contains(value)),
            Self::FieldLessThan { field, value } => event
                .fields
                .get(field)
                .and_then(|candidate| candidate.parse::<f64>().ok())
                .zip(value.parse::<f64>().ok())
                .is_some_and(|(candidate, limit)| candidate < limit),
            Self::FieldGreaterThan { field, value } => event
                .fields
                .get(field)
                .and_then(|candidate| candidate.parse::<f64>().ok())
                .zip(value.parse::<f64>().ok())
                .is_some_and(|(candidate, limit)| candidate > limit),
            Self::HasTag { tag } => event.tags.contains(tag),
        }
    }
}

/// The stateful part of an achievement is intentionally kept separate from
/// the GUI. Definitions can therefore be driven by the same structured event
/// stream as ordinary automation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AchievementMetric {
    EventCount,
    UniqueField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementDefinition {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub on: EventKind,
    #[serde(default)]
    pub when: Vec<Predicate>,
    pub metric: AchievementMetric,
    pub field: Option<String>,
    pub target: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementCatalog {
    #[serde(default)]
    pub achievements: Vec<AchievementDefinition>,
}

impl AchievementCatalog {
    pub fn from_toml(source: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(source)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AchievementUpdate {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub progress: u32,
    pub target: u32,
    pub unlocked: bool,
}

#[derive(Debug, Default)]
pub struct AchievementEvaluator {
    counts: HashMap<String, u32>,
    unique_values: HashMap<String, BTreeSet<String>>,
    unlocked: BTreeSet<String>,
}

impl AchievementEvaluator {
    pub fn observe(
        &mut self,
        definition: &AchievementDefinition,
        event: &AutomationEvent,
    ) -> Option<AchievementUpdate> {
        if definition.target == 0
            || definition.on != event.kind
            || !definition
                .when
                .iter()
                .all(|predicate| predicate.matches(event))
        {
            return None;
        }

        let progress = match definition.metric {
            AchievementMetric::EventCount => {
                let count = self.counts.entry(definition.id.clone()).or_default();
                *count = count.saturating_add(1);
                *count
            }
            AchievementMetric::UniqueField => {
                let field = definition.field.as_deref()?;
                let value = event.fields.get(field).map(String::as_str).map(str::trim)?;
                if value.is_empty() {
                    return None;
                }
                let values = self.unique_values.entry(definition.id.clone()).or_default();
                values.insert(value.to_ascii_uppercase());
                values.len() as u32
            }
        };
        let progress = progress.min(definition.target);
        let unlocked = progress >= definition.target && self.unlocked.insert(definition.id.clone());
        Some(AchievementUpdate {
            id: definition.id.clone(),
            title: definition.title.clone(),
            detail: definition.detail.clone(),
            progress,
            target: definition.target,
            unlocked,
        })
    }

    pub fn is_unlocked(&self, id: &str) -> bool {
        self.unlocked.contains(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ActionTemplate {
    Notify {
        title: String,
        body: String,
        #[serde(default)]
        accent: Option<String>,
    },
    SendExternal {
        source: String,
        target: String,
        message: String,
    },
    ServerSync,
    ServerSendMessage {
        channel: String,
        message: String,
    },
    SetCompose {
        mode: String,
        message: String,
    },
    RadioCommand {
        command: String,
        value: String,
    },
    ReadControl {
        control: String,
        result_key: String,
    },
    WriteControl {
        control: String,
        value: ControlValue,
    },
    ControlSequence {
        name: String,
        steps: Vec<ControlStepTemplate>,
    },
    RequestTransmit {
        mode: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlStepTemplate {
    pub control: String,
    pub value: ControlValue,
    #[serde(default)]
    pub wait_ms: u64,
}

impl ActionTemplate {
    fn render_with_variables(
        &self,
        event: &AutomationEvent,
        variables: &BTreeMap<String, String>,
    ) -> Action {
        let render = |value: &str| render_template(value, event, variables);
        match self {
            Self::Notify {
                title,
                body,
                accent,
            } => Action::Notify {
                title: render(title),
                body: render(body),
                accent: accent.as_deref().map(render),
            },
            Self::SendExternal {
                source,
                target,
                message,
            } => Action::SendExternal {
                source: render(source),
                target: render(target),
                message: render(message),
            },
            Self::ServerSync => Action::ServerSync,
            Self::ServerSendMessage { channel, message } => Action::ServerSendMessage {
                channel: render(channel),
                message: render(message),
            },
            Self::SetCompose { mode, message } => Action::SetCompose {
                mode: render(mode),
                message: render(message),
            },
            Self::RadioCommand { command, value } => Action::RadioCommand {
                command: render(command),
                value: render(value),
            },
            Self::ReadControl {
                control,
                result_key,
            } => Action::ReadControl {
                control: render(control),
                result_key: render(result_key),
            },
            Self::WriteControl { control, value } => Action::WriteControl {
                control: render(control),
                value: render_control_value(value, event, variables),
            },
            Self::ControlSequence { name, steps } => Action::ControlSequence {
                name: render(name),
                steps: steps
                    .iter()
                    .map(|step| ControlStep {
                        control: render(&step.control),
                        value: render_control_value(&step.value, event, variables),
                        wait_ms: step.wait_ms.min(60_000),
                    })
                    .collect(),
            },
            Self::RequestTransmit { mode, message } => Action::RequestTransmit {
                mode: render(mode),
                message: render(message),
            },
        }
    }
}

fn render_control_value(
    value: &ControlValue,
    event: &AutomationEvent,
    variables: &BTreeMap<String, String>,
) -> ControlValue {
    match value {
        ControlValue::Bool(value) => ControlValue::Bool(*value),
        ControlValue::U8(value) => ControlValue::U8(*value),
        ControlValue::I32(value) => ControlValue::I32(*value),
        ControlValue::U64(value) => ControlValue::U64(*value),
        ControlValue::Text(value) => ControlValue::Text(render_template(value, event, variables)),
        ControlValue::RawHex(value) => {
            ControlValue::RawHex(render_template(value, event, variables))
        }
    }
}

fn render_template(
    template: &str,
    event: &AutomationEvent,
    variables: &BTreeMap<String, String>,
) -> String {
    let mut rendered = template
        .replace("${source}", &event.source)
        .replace("${timestamp_ms}", &event.timestamp_ms.to_string());
    for (field, value) in &event.fields {
        rendered = rendered.replace(&format!("${{{field}}}"), value);
    }
    for (name, value) in variables {
        rendered = rendered.replace(&format!("${{{name}}}"), value);
    }
    rendered
}

pub trait Component: Send {
    fn manifest(&self) -> &ComponentManifest;
    fn on_event(&mut self, event: &AutomationEvent) -> Result<Vec<Action>, ComponentError>;

    fn on_event_with_variables(
        &mut self,
        event: &AutomationEvent,
        variables: &BTreeMap<String, String>,
    ) -> Result<Vec<Action>, ComponentError> {
        let _ = variables;
        self.on_event(event)
    }
}

pub struct RuleComponent {
    config: RuleComponentConfig,
}

impl RuleComponent {
    pub fn new(config: RuleComponentConfig) -> Self {
        Self { config }
    }
}

impl Component for RuleComponent {
    fn manifest(&self) -> &ComponentManifest {
        &self.config.component
    }

    fn on_event(&mut self, event: &AutomationEvent) -> Result<Vec<Action>, ComponentError> {
        self.on_event_with_variables(event, &BTreeMap::new())
    }

    fn on_event_with_variables(
        &mut self,
        event: &AutomationEvent,
        variables: &BTreeMap<String, String>,
    ) -> Result<Vec<Action>, ComponentError> {
        if !self.config.component.subscriptions.contains(&event.kind) {
            return Ok(Vec::new());
        }
        Ok(self
            .config
            .rules
            .iter()
            .filter(|rule| rule.on == event.kind)
            .filter(|rule| rule.when.iter().all(|predicate| predicate.matches(event)))
            .flat_map(|rule| {
                rule.actions
                    .iter()
                    .map(|action| action.render_with_variables(event, variables))
            })
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedAction {
    pub component_id: String,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeniedAction {
    pub component_id: String,
    pub capability: Capability,
    pub action: Action,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentOverview {
    pub id: String,
    pub name: String,
    pub subscriptions: Vec<EventKind>,
    pub requested: Vec<Capability>,
    pub granted: Vec<Capability>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DispatchReport {
    pub approved: Vec<ApprovedAction>,
    pub denied: Vec<DeniedAction>,
    pub errors: Vec<String>,
}

#[derive(Default)]
pub struct AutomationHost {
    components: Vec<Box<dyn Component>>,
    grants: HashMap<String, CapabilitySet>,
    variables: BTreeMap<String, String>,
}

impl AutomationHost {
    pub fn component_overview(&self) -> Vec<ComponentOverview> {
        self.components
            .iter()
            .map(|component| {
                let manifest = component.manifest();
                let granted = self
                    .grants
                    .get(&manifest.id)
                    .map(|set| set.0.iter().copied().collect())
                    .unwrap_or_default();
                ComponentOverview {
                    id: manifest.id.clone(),
                    name: manifest.name.clone(),
                    subscriptions: manifest.subscriptions.iter().copied().collect(),
                    requested: manifest.requests.0.iter().copied().collect(),
                    granted,
                }
            })
            .collect()
    }

    pub fn set_variable(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        if !name.trim().is_empty() {
            self.variables.insert(name, value.into());
        }
    }

    pub fn variable(&self, name: &str) -> Option<&str> {
        self.variables.get(name).map(String::as_str)
    }
}

impl AutomationHost {
    pub fn register(&mut self, component: impl Component + 'static) -> Result<(), ComponentError> {
        let id = component.manifest().id.trim();
        if id.is_empty() {
            return Err(ComponentError::InvalidManifest(
                "component id cannot be empty".to_string(),
            ));
        }
        if self
            .components
            .iter()
            .any(|existing| existing.manifest().id == id)
        {
            return Err(ComponentError::DuplicateComponent(id.to_string()));
        }
        self.components.push(Box::new(component));
        Ok(())
    }

    pub fn set_grants(&mut self, component_id: impl Into<String>, grants: CapabilitySet) {
        self.grants.insert(component_id.into(), grants);
    }

    pub fn dispatch(&mut self, event: &AutomationEvent) -> DispatchReport {
        let mut report = DispatchReport::default();
        for component in &mut self.components {
            let manifest = component.manifest().clone();
            let actions = match component.on_event_with_variables(event, &self.variables) {
                Ok(actions) => actions,
                Err(error) => {
                    report.errors.push(format!("{}: {error}", manifest.id));
                    continue;
                }
            };
            let grants = self.grants.get(&manifest.id);
            for action in actions {
                let capability = action.required_capability();
                let requested = manifest.requests.contains(capability);
                let granted = grants.is_some_and(|grants| grants.contains(capability));
                if requested && granted {
                    report.approved.push(ApprovedAction {
                        component_id: manifest.id.clone(),
                        action,
                    });
                } else {
                    report.denied.push(DeniedAction {
                        component_id: manifest.id.clone(),
                        capability,
                        action,
                        reason: if !requested {
                            "capability was not requested in the component manifest".to_string()
                        } else {
                            "capability has not been granted by the operator".to_string()
                        },
                    });
                }
            }
        }
        report
    }
}

#[derive(Debug, Error)]
pub enum ComponentError {
    #[error("invalid component config: {0}")]
    Config(#[from] toml::de::Error),
    #[error("invalid component manifest: {0}")]
    InvalidManifest(String),
    #[error("component already registered: {0}")]
    DuplicateComponent(String),
    #[error("external source error: {0}")]
    External(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component_with(action: ActionTemplate, requests: CapabilitySet) -> RuleComponent {
        RuleComponent::new(RuleComponentConfig {
            component: ComponentManifest {
                id: "spark.callout".to_string(),
                name: "Callsign Spark".to_string(),
                version: "0.1.0".to_string(),
                description: String::new(),
                subscriptions: [EventKind::CallsignHit].into_iter().collect(),
                requests,
            },
            sources: Vec::new(),
            rules: vec![Rule {
                on: EventKind::CallsignHit,
                when: vec![Predicate::HasTag {
                    tag: "directed_to_me".to_string(),
                }],
                actions: vec![action],
            }],
        })
    }

    #[test]
    fn renders_event_fields_into_actions() {
        let mut component = component_with(
            ActionTemplate::Notify {
                title: "Incoming ${call}".to_string(),
                body: "${message}".to_string(),
                accent: Some("magenta".to_string()),
            },
            CapabilitySet::new([Capability::UiNotification]),
        );
        let event = AutomationEvent::new(EventKind::CallsignHit, "ft4")
            .field("call", "W1AW")
            .field("message", "N0ABC W1AW -12")
            .tag("directed_to_me");
        assert_eq!(
            component.on_event(&event).unwrap(),
            vec![Action::Notify {
                title: "Incoming W1AW".to_string(),
                body: "N0ABC W1AW -12".to_string(),
                accent: Some("magenta".to_string()),
            }]
        );
    }

    #[test]
    fn host_requires_manifest_request_and_operator_grant() {
        let component = component_with(
            ActionTemplate::RequestTransmit {
                mode: "FT4".to_string(),
                message: "CQ W1AW FN42".to_string(),
            },
            CapabilitySet::new([Capability::Transmit]),
        );
        let event = AutomationEvent::new(EventKind::CallsignHit, "ft4").tag("directed_to_me");
        let mut host = AutomationHost::default();
        host.register(component).unwrap();

        let denied = host.dispatch(&event);
        assert!(denied.approved.is_empty());
        assert_eq!(denied.denied[0].capability, Capability::Transmit);

        host.set_grants("spark.callout", CapabilitySet::new([Capability::Transmit]));
        let approved = host.dispatch(&event);
        assert_eq!(approved.approved.len(), 1);
    }

    #[test]
    fn parses_discord_and_irc_sources_without_inline_secrets() {
        let source = include_str!("../../../automation.example.toml");
        let config = RuleComponentConfig::from_toml(source).unwrap();
        assert_eq!(config.sources.len(), 2);
        assert_eq!(config.rules.len(), 9);
        assert!(config
            .component
            .subscriptions
            .contains(&EventKind::ContestState));
        assert!(config
            .component
            .subscriptions
            .contains(&EventKind::QsoLogged));
    }

    #[test]
    fn server_publish_requires_its_own_operator_grant() {
        let component = component_with(
            ActionTemplate::ServerSendMessage {
                channel: "ops".to_string(),
                message: "${message}".to_string(),
            },
            CapabilitySet::new([Capability::ServerPublish]),
        );
        let event = AutomationEvent::new(EventKind::CallsignHit, "ft8")
            .field("message", "W1AW N0ABC -10")
            .tag("directed_to_me");
        let mut host = AutomationHost::default();
        host.register(component).unwrap();
        assert_eq!(host.dispatch(&event).denied.len(), 1);

        host.set_grants(
            "spark.callout",
            CapabilitySet::new([Capability::ServerPublish]),
        );
        assert_eq!(host.dispatch(&event).approved.len(), 1);
    }

    #[test]
    fn renders_generic_control_writes_and_bounds_sequence_waits() {
        let mut component = component_with(
            ActionTemplate::ControlSequence {
                name: "low-power-${band}".to_string(),
                steps: vec![ControlStepTemplate {
                    control: "rf_power".to_string(),
                    value: ControlValue::U8(32),
                    wait_ms: 90_000,
                }],
            },
            CapabilitySet::new([Capability::RadioControl]),
        );
        let event = AutomationEvent::new(EventKind::CallsignHit, "test")
            .field("band", "20m")
            .tag("directed_to_me");
        assert_eq!(
            component.on_event(&event).unwrap(),
            vec![Action::ControlSequence {
                name: "low-power-20m".to_string(),
                steps: vec![ControlStep {
                    control: "rf_power".to_string(),
                    value: ControlValue::U8(32),
                    wait_ms: 60_000,
                }],
            }]
        );
    }

    #[test]
    fn parses_control_actions_from_toml_without_vendor_names() {
        let config = RuleComponentConfig::from_toml(
            r#"
[component]
id = "control.guard"
name = "Control guard"
subscriptions = ["radio_state"]
requests = ["radio_control"]

[[rules]]
on = "radio_state"
actions = [
  { action = "read_control", control = "rf_power", result_key = "power" },
  { action = "write_control", control = "tuner", value = { type = "bool", value = false } },
]
"#,
        )
        .expect("control script should parse");
        assert_eq!(config.rules[0].actions.len(), 2);
    }

    #[test]
    fn renders_persistent_variables_into_later_actions() {
        let component = component_with(
            ActionTemplate::WriteControl {
                control: "rf_power".to_string(),
                value: ControlValue::Text("${saved_power}".to_string()),
            },
            CapabilitySet::new([Capability::RadioControl]),
        );
        let event = AutomationEvent::new(EventKind::CallsignHit, "test").tag("directed_to_me");
        let mut host = AutomationHost::default();
        host.register(component).unwrap();
        host.set_grants(
            "spark.callout",
            CapabilitySet::new([Capability::RadioControl]),
        );
        host.set_variable("saved_power", "42");

        let report = host.dispatch(&event);
        assert_eq!(
            report.approved[0].action,
            Action::WriteControl {
                control: "rf_power".to_string(),
                value: ControlValue::Text("42".to_string()),
            }
        );
        assert_eq!(host.variable("saved_power"), Some("42"));
    }

    #[test]
    fn exposes_component_overview_with_requested_and_granted_capabilities() {
        let mut host = AutomationHost::default();
        host.register(component_with(
            ActionTemplate::Notify {
                title: "title".to_string(),
                body: "body".to_string(),
                accent: None,
            },
            CapabilitySet::new([Capability::UiNotification]),
        ))
        .unwrap();

        let overview = host.component_overview();
        assert_eq!(overview.len(), 1);
        assert_eq!(overview[0].id, "spark.callout");
        assert_eq!(overview[0].requested, vec![Capability::UiNotification]);
        assert!(overview[0].granted.is_empty());

        host.set_grants(
            "spark.callout",
            CapabilitySet::new([Capability::UiNotification]),
        );
        assert_eq!(
            host.component_overview()[0].granted,
            vec![Capability::UiNotification]
        );
    }

    #[test]
    fn achievement_evaluator_counts_matching_events_once_per_event() {
        let definition = AchievementDefinition {
            id: "qso-quarter".to_string(),
            title: "QSO Quartermaster".to_string(),
            detail: "Log contacts".to_string(),
            on: EventKind::QsoLogged,
            when: vec![Predicate::FieldEquals {
                field: "mode".to_string(),
                value: "FT8".to_string(),
            }],
            metric: AchievementMetric::EventCount,
            field: None,
            target: 2,
        };
        let mut evaluator = AchievementEvaluator::default();
        let event = AutomationEvent::new(EventKind::QsoLogged, "test").field("mode", "FT8");
        assert_eq!(evaluator.observe(&definition, &event).unwrap().progress, 1);
        let update = evaluator.observe(&definition, &event).unwrap();
        assert!(update.unlocked);
        assert!(evaluator.is_unlocked("qso-quarter"));
        assert!(evaluator
            .observe(&definition, &event)
            .is_some_and(|update| !update.unlocked));
    }

    #[test]
    fn achievement_evaluator_tracks_unique_fields_and_ignores_blank_values() {
        let definition = AchievementDefinition {
            id: "mode-explorer".to_string(),
            title: "Mode Explorer".to_string(),
            detail: "Use two modes".to_string(),
            on: EventKind::QsoLogged,
            when: Vec::new(),
            metric: AchievementMetric::UniqueField,
            field: Some("mode".to_string()),
            target: 2,
        };
        let mut evaluator = AchievementEvaluator::default();
        let blank = AutomationEvent::new(EventKind::QsoLogged, "test").field("mode", " ");
        assert!(evaluator.observe(&definition, &blank).is_none());
        let ft8 = AutomationEvent::new(EventKind::QsoLogged, "test").field("mode", "ft8");
        assert_eq!(evaluator.observe(&definition, &ft8).unwrap().progress, 1);
        let ft4 = AutomationEvent::new(EventKind::QsoLogged, "test").field("mode", "FT4");
        assert!(evaluator.observe(&definition, &ft4).unwrap().unlocked);
    }

    #[test]
    fn parses_the_checked_in_achievement_catalog() {
        let catalog =
            AchievementCatalog::from_toml(include_str!("../../../achievements.example.toml"))
                .expect("achievement catalog should parse");
        assert!(catalog.achievements.len() >= 10);
        assert!(catalog
            .achievements
            .iter()
            .any(|achievement| achievement.id == "state-line"));
    }

    #[test]
    fn numeric_and_negative_predicates_match_structured_fields() {
        let event = AutomationEvent::new(EventKind::QsoLogged, "test")
            .field("snr", "-23")
            .field("country", "JP");
        assert!(Predicate::FieldLessThan {
            field: "snr".to_string(),
            value: "-20".to_string(),
        }
        .matches(&event));
        assert!(Predicate::FieldNotEquals {
            field: "country".to_string(),
            value: "US".to_string(),
        }
        .matches(&event));
    }
}
