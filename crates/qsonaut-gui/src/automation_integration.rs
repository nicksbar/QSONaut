use super::*;

pub(crate) fn parse_automation_hook_detail(detail: &str) -> BTreeMap<String, String> {
    detail
        .split_whitespace()
        .filter_map(|token| {
            let (key, value) = token.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.trim().to_string()))
        })
        .collect()
}

pub(crate) fn normalize_app_event_for_automation(event: AppEvent) -> Option<AutomationEvent> {
    match event {
        AppEvent::ContestProfileChanged {
            enabled,
            operating_mode,
            split_policy,
            fox_hound_role,
        } => Some(
            AutomationEvent::new(EventKind::ContestState, "app.contest_profile")
                .field("enabled", enabled.to_string())
                .field("operating_mode", operating_mode)
                .field("split_policy", split_policy)
                .field("fox_hound_role", fox_hound_role),
        ),
        AppEvent::CallsignHit {
            mode,
            call,
            snr_db,
            freq_hz,
            message,
            directed_to_me,
        } => {
            let event = AutomationEvent::new(EventKind::CallsignHit, "app.callsign_hit")
                .field("mode", mode)
                .field("call", call)
                .field("snr", format!("{snr_db:+.1}"))
                .field("freq_hz", freq_hz.to_string())
                .field("message", message)
                .field("directed_to_me", directed_to_me.to_string());
            Some(if directed_to_me {
                event.tag("directed_to_me")
            } else {
                event
            })
        }
        AppEvent::QsoLogged {
            mode,
            call,
            band,
            frequency_hz,
        } => Some(
            AutomationEvent::new(EventKind::QsoLogged, "app.qso_log")
                .field("mode", mode)
                .field("call", call)
                .field("band", band)
                .field("frequency_hz", frequency_hz.to_string()),
        ),
        AppEvent::ExternalMessageReceived {
            source,
            author,
            message,
            channel,
        } => Some(
            AutomationEvent::new(EventKind::ExternalMessage, source.clone())
                .field("source", source)
                .field("author", author)
                .field("message", message)
                .field("channel", channel),
        ),
        AppEvent::ServerMessageReceived { kind, fields } => {
            let mut event = AutomationEvent::new(EventKind::ServerMessage, "qsonaut-server")
                .field("kind", kind.clone())
                .tag(kind);
            for (key, value) in fields {
                event = event.field(key, value);
            }
            Some(event)
        }
        AppEvent::AutomationHook {
            kind,
            source,
            detail,
        } => {
            let event_kind = match kind.as_str() {
                "contest_state" => EventKind::ContestState,
                "operator_profile" => EventKind::OperatorProfile,
                "callsign_hit" => EventKind::CallsignHit,
                "qso_logged" => EventKind::QsoLogged,
                "radio_state" => EventKind::RadioState,
                _ => return None,
            };
            let mut event = AutomationEvent::new(event_kind, source)
                .field("kind", kind)
                .field("detail", detail.clone());
            for (key, value) in parse_automation_hook_detail(&detail) {
                event = event.field(key, value);
            }
            Some(event)
        }
        _ => None,
    }
}

pub(crate) fn external_source_transport(source: &str) -> Option<String> {
    let (transport, _) = source.trim().split_once(':')?;
    let transport = transport.trim();
    if transport.is_empty() {
        None
    } else {
        Some(transport.to_ascii_lowercase())
    }
}

pub(crate) fn configured_external_transports(config: &RuleComponentConfig) -> HashSet<String> {
    config
        .sources
        .iter()
        .map(|source| match source {
            ExternalSourceConfig::Discord { .. } => "discord",
            ExternalSourceConfig::Irc { .. } => "irc",
        })
        .map(str::to_string)
        .collect()
}

pub(crate) fn bootstrap_automation_host() -> (AutomationHost, String, HashSet<String>) {
    let mut host = AutomationHost::default();
    let source = include_str!("../../../automation.example.toml");

    match RuleComponentConfig::from_toml(source) {
        Ok(config) => {
            let configured_transports = configured_external_transports(&config);
            let component_id = config.component.id.clone();
            if let Err(error) = host.register(RuleComponent::new(config)) {
                return (
                    host,
                    format!("Automation host active, component registration failed: {error}"),
                    HashSet::new(),
                );
            }

            let external_send_enabled = parse_bool_env("QSONAUT_AUTOMATION_ENABLE_EXTERNAL_SEND");
            let server_publish_enabled = parse_bool_env("QSONAUT_AUTOMATION_ENABLE_SERVER_PUBLISH");
            let mut grants = vec![Capability::UiNotification, Capability::ServerRead];
            if external_send_enabled {
                grants.push(Capability::ExternalSend);
            }
            if server_publish_enabled {
                grants.push(Capability::ServerPublish);
            }
            let grants = CapabilitySet::new(grants);
            host.set_grants(component_id.clone(), grants);

            let mut grant_status = vec!["ui_notification", "server_read"];
            if external_send_enabled {
                grant_status.push("external_send (env-enabled)");
            }
            if server_publish_enabled {
                grant_status.push("server_publish (env-enabled)");
            }
            (
                host,
                format!(
                    "Automation component loaded: {component_id} (granted: {})",
                    grant_status.join(", ")
                ),
                configured_transports,
            )
        }
        Err(error) => (
            host,
            format!("Automation config parse failed; runtime hooks paused: {error}"),
            HashSet::new(),
        ),
    }
}
