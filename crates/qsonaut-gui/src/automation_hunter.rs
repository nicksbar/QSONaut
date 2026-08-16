use super::*;

#[derive(Debug, Clone)]
pub(super) struct ExternalSendRecord {
    pub(super) utc: String,
    pub(super) source: String,
    pub(super) target: String,
    pub(super) message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AchievementKind {
    FirstDecode,
    DirectedCall,
    FirstQsoLogged,
    TenQsosLogged,
    FiftyQsosLogged,
    DupeShield,
    CenturyHunter,
}

impl AchievementKind {
    const ALL: [Self; 7] = [
        Self::FirstDecode,
        Self::DirectedCall,
        Self::FirstQsoLogged,
        Self::TenQsosLogged,
        Self::FiftyQsosLogged,
        Self::DupeShield,
        Self::CenturyHunter,
    ];

    fn presentation(self) -> (&'static str, &'static str) {
        match self {
            Self::FirstDecode => ("Signal Hunter", "Capture the first decode burst"),
            Self::DirectedCall => ("You Have Mail", "Receive a directed callsign hit"),
            Self::FirstQsoLogged => ("Logbook Opened", "Log the first contact"),
            Self::TenQsosLogged => ("Ragchew Rookie", "Log 10 contacts"),
            Self::FiftyQsosLogged => ("Pileup Wrangler", "Log 50 contacts"),
            Self::DupeShield => ("Dupe Shield", "Prevent 10 duplicate TX attempts"),
            Self::CenturyHunter => ("Century Hunter", "Hear 100 unique callsigns"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum HunterMetric {
    UniqueHeard,
    DirectedHits,
    QsoLogged,
    DupeBlocks,
    DecodeBursts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CustomAchievementRule {
    id: String,
    title: String,
    detail: String,
    metric: HunterMetric,
    threshold: u32,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    unlocked: bool,
}

#[derive(Debug, Clone)]
pub(super) struct HunterAlert {
    pub(super) utc: String,
    pub(super) title: String,
    pub(super) detail: String,
    pub(super) accent: Color32,
}

impl QsonautGuiApp {
    pub(super) fn push_hunter_alert(
        &mut self,
        title: impl Into<String>,
        detail: impl Into<String>,
        accent: Color32,
    ) {
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or_default();
        self.hunter_feed.push_back(HunterAlert {
            utc: utc_hhmmss_millis(now_s),
            title: title.into(),
            detail: detail.into(),
            accent,
        });
        while self.hunter_feed.len() > 24 {
            self.hunter_feed.pop_front();
        }
    }

    pub(super) fn unlock_achievement(
        &mut self,
        kind: AchievementKind,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) {
        if self.hunter_unlocked.insert(kind) {
            let title = title.into();
            let detail = detail.into();
            self.push_hunter_alert(
                format!("🏆 Achievement unlocked: {title}"),
                detail,
                Color32::from_rgb(255, 201, 92),
            );
            self.profile_dirty = true;
            self.persist_profile("Auto-saved");
        }
    }

    fn custom_hunter_metric_value(&self, metric: HunterMetric) -> u32 {
        match metric {
            HunterMetric::UniqueHeard => self.hunter_unique_heard.len() as u32,
            HunterMetric::DirectedHits => self.hunter_directed_hits,
            HunterMetric::QsoLogged => self.qso_log.contacts.len() as u32,
            HunterMetric::DupeBlocks => self.hunter_dupe_blocks,
            HunterMetric::DecodeBursts => self.hunter_decode_bursts,
        }
    }

    fn evaluate_custom_hunter_rules(&mut self) {
        let mut newly_unlocked = Vec::new();
        for (idx, rule) in self.hunter_custom_rules.iter().enumerate() {
            if rule.enabled && !rule.unlocked {
                let progress = self.custom_hunter_metric_value(rule.metric);
                if progress >= rule.threshold {
                    newly_unlocked.push(idx);
                }
            }
        }

        if newly_unlocked.is_empty() {
            return;
        }

        for idx in newly_unlocked {
            if let Some(rule) = self.hunter_custom_rules.get_mut(idx) {
                let title = rule.title.clone();
                let detail = rule.detail.clone();
                rule.unlocked = true;
                self.push_hunter_alert(
                    format!("🏆 Achievement unlocked: {title}"),
                    detail,
                    Color32::from_rgb(132, 228, 255),
                );
            }
        }
        self.profile_dirty = true;
        self.persist_profile("Auto-saved");
    }

    fn add_custom_hunter_rule(&mut self) {
        let title = self.hunter_custom_title_input.trim();
        let detail = self.hunter_custom_detail_input.trim();
        if title.is_empty() || detail.is_empty() || self.hunter_custom_threshold_input == 0 {
            self.push_hunter_alert(
                "⚠ Custom achievement not saved",
                "Provide a title, detail, and a threshold greater than zero.",
                Color32::from_rgb(255, 170, 75),
            );
            return;
        }

        let mut id = title
            .chars()
            .map(|ch: char| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        if id.is_empty() {
            id = format!("custom-{}", self.hunter_custom_rules.len() + 1);
        }
        if self.hunter_custom_rules.iter().any(|rule| rule.id == id) {
            id = format!("{id}-{}", self.hunter_custom_rules.len() + 1);
        }

        self.hunter_custom_rules.push(CustomAchievementRule {
            id,
            title: title.to_string(),
            detail: detail.to_string(),
            metric: self.hunter_custom_metric_input,
            threshold: self.hunter_custom_threshold_input.max(1),
            enabled: self.hunter_custom_enabled_input,
            unlocked: false,
        });
        self.hunter_custom_title_input.clear();
        self.hunter_custom_detail_input.clear();
        self.hunter_custom_metric_input = HunterMetric::UniqueHeard;
        self.hunter_custom_threshold_input = 1;
        self.hunter_custom_enabled_input = true;
        self.profile_dirty = true;
        self.persist_profile("Auto-saved");
    }

    fn remove_custom_hunter_rule(&mut self, index: usize) {
        if index < self.hunter_custom_rules.len() {
            self.hunter_custom_rules.remove(index);
            self.profile_dirty = true;
            self.persist_profile("Auto-saved");
        }
    }

    fn track_hunter_event(&mut self, event: &AppEvent) {
        match event {
            AppEvent::CallsignHit {
                call,
                directed_to_me,
                ..
            } => {
                let call = call.trim().to_ascii_uppercase();
                if !call.is_empty() {
                    self.hunter_unique_heard.insert(call);
                    if self.hunter_unique_heard.len() >= 100 {
                        self.unlock_achievement(
                            AchievementKind::CenturyHunter,
                            "Century Hunter",
                            "Heard 100 unique callsigns in this session",
                        );
                    }
                }
                if *directed_to_me {
                    self.hunter_directed_hits = self.hunter_directed_hits.saturating_add(1);
                    self.unlock_achievement(
                        AchievementKind::DirectedCall,
                        "You Have Mail",
                        "Received your first directed-on-you callsign hit",
                    );
                }
            }
            AppEvent::QsoLogged { .. } => {
                let qso_count = self.qso_log.contacts.len();
                if qso_count >= 1 {
                    self.unlock_achievement(
                        AchievementKind::FirstQsoLogged,
                        "Logbook Opened",
                        "Logged your first contact",
                    );
                }
                if qso_count >= 10 {
                    self.unlock_achievement(
                        AchievementKind::TenQsosLogged,
                        "Ragchew Rookie",
                        "Logged 10 contacts",
                    );
                }
                if qso_count >= 50 {
                    self.unlock_achievement(
                        AchievementKind::FiftyQsosLogged,
                        "Pileup Wrangler",
                        "Logged 50 contacts",
                    );
                }
            }
            _ => {}
        }

        self.evaluate_custom_hunter_rules();
    }

    pub(super) fn track_decode_batch(&mut self, decode_count: usize) {
        if decode_count > 0 {
            self.hunter_decode_bursts = self.hunter_decode_bursts.saturating_add(1);
            self.unlock_achievement(
                AchievementKind::FirstDecode,
                "Signal Hunter",
                "Captured your first decode burst in this session",
            );
            self.evaluate_custom_hunter_rules();
        }
    }

    pub(super) fn draw_hunter_panel(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let worked_unique = self
            .qso_log
            .contacts
            .iter()
            .map(|contact| contact.callsign.trim().to_ascii_uppercase())
            .collect::<HashSet<_>>()
            .len();

        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("🏆 Achievement Hunter")
                    .strong()
                    .color(Color32::from_rgb(255, 201, 92)),
            );
            ui.separator();
            ui.label(format!("Unlocked: {}", self.hunter_unlocked.len()));
            ui.separator();
            ui.label(format!("Unique heard: {}", self.hunter_unique_heard.len()));
            ui.separator();
            ui.label(format!("Worked calls: {worked_unique}"));
            ui.separator();
            ui.label(format!("Dupe saves: {}", self.hunter_dupe_blocks));
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "Band {}",
                    snapshot
                        .frequency_hz
                        .map(band_for_frequency)
                        .filter(|band| !band.is_empty())
                        .unwrap_or("?")
                ))
                .monospace(),
            );
        });

        if let Some(alert) = self.hunter_feed.back() {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{} · {} — {}",
                    alert.utc, alert.title, alert.detail
                ))
                .small()
                .color(alert.accent),
            );
        }

        ui.add_space(8.0);
        ui.label(
            RichText::new("Built-in achievements")
                .strong()
                .color(Color32::from_rgb(255, 201, 92)),
        );
        let qso_count = self.qso_log.contacts.len() as u32;
        for achievement in AchievementKind::ALL {
            let (title, detail) = achievement.presentation();
            let (progress, target) = match achievement {
                AchievementKind::FirstDecode => (self.hunter_decode_bursts.min(1), 1),
                AchievementKind::DirectedCall => (self.hunter_directed_hits.min(1), 1),
                AchievementKind::FirstQsoLogged => (qso_count.min(1), 1),
                AchievementKind::TenQsosLogged => (qso_count.min(10), 10),
                AchievementKind::FiftyQsosLogged => (qso_count.min(50), 50),
                AchievementKind::DupeShield => (self.hunter_dupe_blocks.min(10), 10),
                AchievementKind::CenturyHunter => {
                    ((self.hunter_unique_heard.len() as u32).min(100), 100)
                }
            };
            let unlocked = self.hunter_unlocked.contains(&achievement);
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(if unlocked { "🏆" } else { "🔒" })
                            .color(if unlocked {
                                Color32::from_rgb(255, 201, 92)
                            } else {
                                Color32::GRAY
                            }),
                    );
                    ui.label(RichText::new(title).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(if unlocked { "UNLOCKED" } else { "IN PROGRESS" })
                                .small()
                                .color(if unlocked {
                                    Color32::LIGHT_GREEN
                                } else {
                                    Color32::GRAY
                                }),
                        );
                    });
                });
                ui.label(RichText::new(detail).small().color(Color32::GRAY));
                ui.add(
                    egui::ProgressBar::new(progress as f32 / target as f32)
                        .text(format!("{progress} / {target}")),
                );
            });
            ui.add_space(3.0);
        }

        ui.add_space(4.0);
        ui.label(RichText::new("Recent achievement activity").strong());
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                if self.hunter_feed.is_empty() {
                    ui.label(
                        RichText::new("No hunter alerts yet — spin the dial and chase one!")
                            .small()
                            .color(Color32::GRAY),
                    );
                    return;
                }

                for alert in self.hunter_feed.iter().rev().take(6) {
                    ui.label(
                        RichText::new(format!("{}  {}", alert.utc, alert.title))
                            .small()
                            .color(alert.accent),
                    );
                    ui.label(RichText::new(&alert.detail).small().color(Color32::GRAY));
                    ui.add_space(2.0);
                }
            });

        ui.separator();
        ui.label(
            RichText::new("Custom achievements")
                .strong()
                .color(Color32::from_rgb(132, 228, 255)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Title");
            ui.text_edit_singleline(&mut self.hunter_custom_title_input);
            ui.label("Detail");
            ui.text_edit_singleline(&mut self.hunter_custom_detail_input);
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Metric");
            egui::ComboBox::from_id_salt("hunter_custom_metric")
                .selected_text(match self.hunter_custom_metric_input {
                    HunterMetric::UniqueHeard => "Unique heard",
                    HunterMetric::DirectedHits => "Directed hits",
                    HunterMetric::QsoLogged => "QSOs logged",
                    HunterMetric::DupeBlocks => "Dupe blocks",
                    HunterMetric::DecodeBursts => "Decode bursts",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.hunter_custom_metric_input,
                        HunterMetric::UniqueHeard,
                        "Unique heard",
                    );
                    ui.selectable_value(
                        &mut self.hunter_custom_metric_input,
                        HunterMetric::DirectedHits,
                        "Directed hits",
                    );
                    ui.selectable_value(
                        &mut self.hunter_custom_metric_input,
                        HunterMetric::QsoLogged,
                        "QSOs logged",
                    );
                    ui.selectable_value(
                        &mut self.hunter_custom_metric_input,
                        HunterMetric::DupeBlocks,
                        "Dupe blocks",
                    );
                    ui.selectable_value(
                        &mut self.hunter_custom_metric_input,
                        HunterMetric::DecodeBursts,
                        "Decode bursts",
                    );
                });
            ui.label("Threshold");
            ui.add(egui::DragValue::new(&mut self.hunter_custom_threshold_input).range(1..=10_000));
            ui.checkbox(&mut self.hunter_custom_enabled_input, "Enabled");
            if ui.button("Add custom achievement").clicked() {
                self.add_custom_hunter_rule();
            }
        });

        ui.add_space(4.0);
        if self.hunter_custom_rules.is_empty() {
            ui.label(
                RichText::new("No custom achievements yet — add one to make the chase personal.")
                    .small()
                    .color(Color32::GRAY),
            );
        } else {
            let mut remove_idx = None;
            let unique_heard = self.hunter_unique_heard.len() as u32;
            let directed_hits = self.hunter_directed_hits;
            let qso_logged = self.qso_log.contacts.len() as u32;
            let dupe_blocks = self.hunter_dupe_blocks;
            let decode_bursts = self.hunter_decode_bursts;
            for (idx, rule) in self.hunter_custom_rules.iter_mut().enumerate() {
                let progress = match rule.metric {
                    HunterMetric::UniqueHeard => unique_heard,
                    HunterMetric::DirectedHits => directed_hits,
                    HunterMetric::QsoLogged => qso_logged,
                    HunterMetric::DupeBlocks => dupe_blocks,
                    HunterMetric::DecodeBursts => decode_bursts,
                };
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut rule.enabled, "");
                        ui.label(RichText::new(&rule.title).strong().color(Color32::WHITE));
                        if rule.unlocked {
                            ui.label(
                                RichText::new("UNLOCKED")
                                    .small()
                                    .color(Color32::from_rgb(132, 228, 255)),
                            );
                        }
                        if ui.small_button("Remove").clicked() {
                            remove_idx = Some(idx);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&rule.detail).small().color(Color32::GRAY));
                        ui.separator();
                        ui.label(
                            RichText::new(format!("{} / {}", progress, rule.threshold))
                                .small()
                                .monospace(),
                        );
                    });
                });
                ui.add_space(4.0);
            }
            if let Some(idx) = remove_idx {
                self.remove_custom_hunter_rule(idx);
            }
        }
    }

    fn execute_automation_external_send(
        &mut self,
        source: &str,
        target: &str,
        message: &str,
    ) -> String {
        let source = source.trim();
        let target = target.trim();
        let message = message.trim();

        if source.is_empty() || target.is_empty() || message.is_empty() {
            return "External send rejected: source, target, and message are required".to_string();
        }

        let Some(transport) = external_source_transport(source) else {
            return format!(
                "External send rejected: source '{source}' must use '<transport>:<id>'"
            );
        };
        if !self.automation_external_transports.contains(&transport) {
            let mut configured: Vec<_> = self
                .automation_external_transports
                .iter()
                .cloned()
                .collect();
            configured.sort();
            let known = if configured.is_empty() {
                "none".to_string()
            } else {
                configured.join(",")
            };
            return format!(
                "External send rejected: transport '{transport}' is not configured (known: {known})"
            );
        }

        self.automation_external_outbox
            .push_back(ExternalSendRecord {
                utc: utc_hhmmss_millis(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|duration| duration.as_secs_f64())
                        .unwrap_or_default(),
                ),
                source: source.to_string(),
                target: target.to_string(),
                message: message.to_string(),
            });
        while self.automation_external_outbox.len() > 32 {
            self.automation_external_outbox.pop_front();
        }

        format!(
            "Queued external send via {source} -> {target} ({} chars)",
            message.chars().count()
        )
    }

    pub(super) fn pump_automation_events(&mut self) {
        loop {
            match self.automation_event_rx.try_recv() {
                Ok(app_event) => {
                    self.track_hunter_event(&app_event);
                    let Some(event) = normalize_app_event_for_automation(app_event) else {
                        continue;
                    };
                    let report = self.automation_host.dispatch(&event);

                    for approved in &report.approved {
                        match &approved.action {
                            Action::Notify {
                                title,
                                body,
                                accent,
                            } => {
                                let accent = accent.as_deref().unwrap_or("default");
                                self.automation_status =
                                    format!("🤖 {title} — {body} (accent: {accent})");
                            }
                            Action::SetCompose { mode, message } => {
                                let normalized_mode = mode.trim().to_ascii_uppercase();
                                if normalized_mode == "FT8" {
                                    self.ft8_compose = message.clone();
                                    self.automation_status =
                                        "🤖 Automation prepared FT8 compose text".to_string();
                                } else {
                                    self.digital_compose = message.clone();
                                    self.automation_status = format!(
                                        "🤖 Automation prepared {} compose text",
                                        normalized_mode
                                    );
                                }
                            }
                            Action::SendExternal {
                                source,
                                target,
                                message,
                            } => {
                                self.automation_status = format!(
                                    "🤖 {}",
                                    self.execute_automation_external_send(source, target, message)
                                );
                            }
                            Action::ServerSync => {
                                self.automation_status = if let Some(client) = &self.server_client {
                                    client.request_sync();
                                    "🤖 Requested QSONaut Server sync".to_string()
                                } else {
                                    "🤖 Server sync unavailable: no configured connection"
                                        .to_string()
                                };
                            }
                            Action::ServerSendMessage { channel, message } => {
                                self.automation_status = if channel.trim().is_empty()
                                    || message.trim().is_empty()
                                {
                                    "🤖 Server publish rejected: channel and message are required"
                                        .to_string()
                                } else if channel.chars().count() > 80
                                    || message.chars().count() > 2_000
                                {
                                    "🤖 Server publish rejected: message exceeds server limits"
                                        .to_string()
                                } else if let Some(client) = &self.server_client {
                                    client.publish_channel_message(channel, message);
                                    format!("🤖 Published automation message to #{channel}")
                                } else {
                                    "🤖 Server publish unavailable: no configured connection"
                                        .to_string()
                                };
                            }
                            Action::RadioCommand { command, value } => {
                                self.automation_status = format!(
                                    "🤖 {}",
                                    self.execute_automation_radio_command(command, value)
                                );
                            }
                            Action::RequestTransmit { mode, message } => {
                                self.automation_status = format!(
                                    "🤖 {}",
                                    self.execute_automation_transmit_request(mode, message)
                                );
                            }
                        }
                    }

                    if !report.denied.is_empty() {
                        self.automation_status = format!(
                            "🤖 Automation denied {} action(s) (capability not granted/requested)",
                            report.denied.len()
                        );
                    }
                    if let Some(error) = report.errors.first() {
                        self.automation_status = format!("🤖 Automation component error: {error}");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(skipped)) => {
                    self.automation_status =
                        format!("🤖 Automation event stream lagged; skipped {skipped} event(s)");
                }
                Err(TryRecvError::Closed) => {
                    self.automation_status =
                        "🤖 Automation event stream closed; restart required".to_string();
                    break;
                }
            }
        }
    }

    pub(super) fn pump_server_automation_events(&self) {
        let Some(client) = &self.server_client else {
            return;
        };
        for event in client.drain_automation_events() {
            self.app_events.publish(AppEvent::ServerMessageReceived {
                kind: event.kind,
                fields: event.fields,
            });
        }
    }

    fn execute_automation_radio_command(&mut self, command: &str, value: &str) -> String {
        let snapshot = self.state.lock().expect("ui state lock poisoned").clone();
        if snapshot.ptt_on
            || self.ft8_tx_active.load(Ordering::Acquire)
            || self.digital_tx_active.load(Ordering::Acquire)
        {
            return "Radio command blocked: transmitter is currently active".to_string();
        }

        match command.trim().to_ascii_lowercase().as_str() {
            "tune_delta_hz" => match value.trim().parse::<i64>() {
                Ok(delta_hz) => {
                    self.send_command(GuiCommand::TuneDelta(delta_hz));
                    format!("Applied radio tune delta of {delta_hz} Hz")
                }
                Err(_) => format!("Rejected radio command: invalid tune delta '{value}'"),
            },
            "set_filter"
                if !find_model(&self.config.radio.model).is_some_and(|profile| {
                    matches!(profile.protocol, Protocol::IcomCiV { .. })
                }) =>
            {
                "Rejected radio command: selected profile has no CI-V filter control".to_string()
            }
            "set_filter" => match value.trim().parse::<u8>() {
                Ok(filter @ 1..=3) => {
                    self.send_command(GuiCommand::SetFilter(filter));
                    format!("Applied radio filter FIL{filter}")
                }
                Ok(other) => {
                    format!("Rejected radio command: filter {other} is outside 1..=3")
                }
                Err(_) => format!("Rejected radio command: invalid filter '{value}'"),
            },
            "tune_workspace_band_hz" => match value.trim().parse::<u64>() {
                Ok(frequency_hz) if frequency_hz > 0 => {
                    self.send_command(GuiCommand::ApplyWorkspace {
                        mode: self.workspace_mode,
                        frequency_hz,
                    });
                    format!(
                        "Applied workspace band tune to {:.3} MHz",
                        frequency_hz as f64 / 1_000_000.0
                    )
                }
                Ok(_) => "Rejected radio command: frequency must be > 0 Hz".to_string(),
                Err(_) => format!("Rejected radio command: invalid frequency '{value}'"),
            },
            "cycle_mode" => {
                self.send_command(GuiCommand::CycleMode);
                "Applied radio mode cycle".to_string()
            }
            blocked @ "set_ptt" | blocked @ "toggle_ptt" => {
                format!("Rejected radio command: {blocked} is TX-controlled and not allowed")
            }
            other => format!("Rejected radio command: unsupported command '{other}'"),
        }
    }

    fn execute_automation_transmit_request(&mut self, mode: &str, message: &str) -> String {
        let snapshot = self.state.lock().expect("ui state lock poisoned").clone();
        if !self.any_tx_armed(&snapshot) {
            return "TX request blocked: all transmit paths are disarmed".to_string();
        }
        if snapshot.ptt_on
            || self.ft8_tx_active.load(Ordering::Acquire)
            || self.digital_tx_active.load(Ordering::Acquire)
        {
            return "TX request blocked: transmitter is currently active".to_string();
        }

        let Some(parsed_mode) = parse_workspace_mode_token(mode) else {
            return format!("TX request rejected: unknown mode '{mode}'");
        };
        let trimmed_message = message.trim();
        if trimmed_message.is_empty() {
            return "TX request rejected: message is empty".to_string();
        }

        match parsed_mode {
            WorkspaceMode::Ft8 => {
                self.ft8_compose = trimmed_message.to_string();
                self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::Standard, None);
                format!("FT8 TX request accepted: {}", self.ft8_seq_status)
            }
            mode if workspace_mode_supports_native_tx(mode) => {
                self.digital_compose = trimmed_message.to_string();
                self.queue_native_digital_tx(mode);
                format!(
                    "{} TX request accepted: {}",
                    mode.label(),
                    self.digital_tx_status
                )
            }
            unsupported => format!(
                "TX request rejected: {} transmit path is not available",
                unsupported.label()
            ),
        }
    }
}
