//! Event-driven, permission-gated automation for RigForge.
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
    Command,
    ExternalMessage,
    Timer,
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
    SetCompose,
    RadioControl,
    Transmit,
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
    SetCompose {
        mode: String,
        message: String,
    },
    RadioCommand {
        command: String,
        value: String,
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
            Self::SetCompose { .. } => Capability::SetCompose,
            Self::RadioCommand { .. } => Capability::RadioControl,
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
    FieldContains { field: String, value: String },
    HasTag { tag: String },
}

impl Predicate {
    fn matches(&self, event: &AutomationEvent) -> bool {
        match self {
            Self::FieldEquals { field, value } => event.fields.get(field) == Some(value),
            Self::FieldContains { field, value } => event
                .fields
                .get(field)
                .is_some_and(|candidate| candidate.contains(value)),
            Self::HasTag { tag } => event.tags.contains(tag),
        }
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
    SetCompose {
        mode: String,
        message: String,
    },
    RadioCommand {
        command: String,
        value: String,
    },
    RequestTransmit {
        mode: String,
        message: String,
    },
}

impl ActionTemplate {
    fn render(&self, event: &AutomationEvent) -> Action {
        let render = |value: &str| render_template(value, event);
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
            Self::SetCompose { mode, message } => Action::SetCompose {
                mode: render(mode),
                message: render(message),
            },
            Self::RadioCommand { command, value } => Action::RadioCommand {
                command: render(command),
                value: render(value),
            },
            Self::RequestTransmit { mode, message } => Action::RequestTransmit {
                mode: render(mode),
                message: render(message),
            },
        }
    }
}

fn render_template(template: &str, event: &AutomationEvent) -> String {
    let mut rendered = template
        .replace("${source}", &event.source)
        .replace("${timestamp_ms}", &event.timestamp_ms.to_string());
    for (field, value) in &event.fields {
        rendered = rendered.replace(&format!("${{{field}}}"), value);
    }
    rendered
}

pub trait Component: Send {
    fn manifest(&self) -> &ComponentManifest;
    fn on_event(&mut self, event: &AutomationEvent) -> Result<Vec<Action>, ComponentError>;
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
        if !self.config.component.subscriptions.contains(&event.kind) {
            return Ok(Vec::new());
        }
        Ok(self
            .config
            .rules
            .iter()
            .filter(|rule| rule.on == event.kind)
            .filter(|rule| rule.when.iter().all(|predicate| predicate.matches(event)))
            .flat_map(|rule| rule.actions.iter().map(|action| action.render(event)))
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
            let actions = match component.on_event(event) {
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
        assert_eq!(config.rules.len(), 2);
    }
}
