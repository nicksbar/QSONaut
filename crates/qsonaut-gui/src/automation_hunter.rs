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
    BandCollector,
    GridMapper,
    DXChaser,
    ModeExplorer,
    EarlyBird,
    NightOwl,
    ContestOperator,
    AudioAlchemist,
    SignalSurvivor,
    QsoQuarter,
    ModeCartographer,
    StateLine,
    Ft8Pathfinder,
    Ft4Pathfinder,
    CwOperator,
}

impl AchievementKind {
    const ALL: [Self; 22] = [
        Self::FirstDecode,
        Self::DirectedCall,
        Self::FirstQsoLogged,
        Self::TenQsosLogged,
        Self::FiftyQsosLogged,
        Self::DupeShield,
        Self::CenturyHunter,
        Self::BandCollector,
        Self::GridMapper,
        Self::DXChaser,
        Self::ModeExplorer,
        Self::EarlyBird,
        Self::NightOwl,
        Self::ContestOperator,
        Self::AudioAlchemist,
        Self::SignalSurvivor,
        Self::QsoQuarter,
        Self::ModeCartographer,
        Self::StateLine,
        Self::Ft8Pathfinder,
        Self::Ft4Pathfinder,
        Self::CwOperator,
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
            Self::BandCollector => ("Band Collector", "Work contacts on 5 different bands"),
            Self::GridMapper => ("Grid Mapper", "Work 25 distinct grid squares"),
            Self::DXChaser => ("DX Chaser", "Work a station outside your home country"),
            Self::ModeExplorer => ("Mode Explorer", "Log contacts in 3 digital modes"),
            Self::EarlyBird => ("Early Bird", "Log a QSO before 07:00 UTC"),
            Self::NightOwl => ("Night Owl", "Log a QSO after 23:00 UTC"),
            Self::ContestOperator => ("Contest Operator", "Complete a contest exchange"),
            Self::AudioAlchemist => ("Audio Alchemist", "Decode 1000 signal bursts"),
            Self::SignalSurvivor => ("Signal Survivor", "Log a contact below -20 dB"),
            Self::QsoQuarter => ("QSO Quartermaster", "Log 25 contacts"),
            Self::ModeCartographer => ("Mode Cartographer", "Log contacts in 3 different modes"),
            Self::StateLine => ("Worked All States", "Work all 50 US states"),
            Self::Ft8Pathfinder => ("FT8 Pathfinder", "Log 25 FT8 contacts"),
            Self::Ft4Pathfinder => ("FT4 Pathfinder", "Log 25 FT4 contacts"),
            Self::CwOperator => ("CW Operator", "Log 10 CW contacts"),
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

fn custom_rule_id(title: &str, existing: &[CustomAchievementRule]) -> String {
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
        id = format!("custom-{}", existing.len() + 1);
    }
    if existing.iter().any(|rule| rule.id == id) {
        id = format!("{id}-{}", existing.len() + 1);
    }
    id
}

fn achievement_progress(
    achievement: AchievementKind,
    qso_count: u32,
    unique_heard: u32,
    dupe_blocks: u32,
    decode_bursts: u32,
    contacts: &[QsoRecord],
) -> (u32, u32) {
    match achievement {
        AchievementKind::FirstDecode => (decode_bursts.min(1), 1),
        AchievementKind::DirectedCall => (0, 1),
        AchievementKind::FirstQsoLogged => (qso_count.min(1), 1),
        AchievementKind::TenQsosLogged => (qso_count.min(10), 10),
        AchievementKind::FiftyQsosLogged => (qso_count.min(50), 50),
        AchievementKind::DupeShield => (dupe_blocks.min(10), 10),
        AchievementKind::CenturyHunter => (unique_heard.min(100), 100),
        AchievementKind::BandCollector => {
            let bands = contacts
                .iter()
                .map(|contact| contact.band.trim())
                .filter(|band| !band.is_empty())
                .collect::<HashSet<_>>()
                .len() as u32;
            (bands.min(5), 5)
        }
        AchievementKind::GridMapper => {
            let grids = contacts
                .iter()
                .map(|contact| contact.grid.trim())
                .filter(|grid| !grid.is_empty())
                .collect::<HashSet<_>>()
                .len() as u32;
            (grids.min(25), 25)
        }
        AchievementKind::DXChaser => (0, 1),
        AchievementKind::ModeExplorer => {
            let modes = contacts
                .iter()
                .map(|contact| contact.mode.trim())
                .filter(|mode| !mode.is_empty())
                .collect::<HashSet<_>>()
                .len() as u32;
            (modes.min(3), 3)
        }
        AchievementKind::EarlyBird
        | AchievementKind::NightOwl
        | AchievementKind::ContestOperator
        | AchievementKind::SignalSurvivor => (0, 1),
        AchievementKind::AudioAlchemist => (decode_bursts.min(1_000), 1_000),
        AchievementKind::QsoQuarter => (qso_count.min(25), 25),
        AchievementKind::ModeCartographer => {
            let modes = contacts
                .iter()
                .map(|contact| contact.mode.trim())
                .filter(|mode| !mode.is_empty())
                .collect::<HashSet<_>>()
                .len() as u32;
            (modes.min(3), 3)
        }
        AchievementKind::StateLine => {
            let states = contacts
                .iter()
                .map(|contact| contact.state.trim())
                .filter(|state| !state.is_empty())
                .collect::<HashSet<_>>()
                .len() as u32;
            (states.min(50), 50)
        }
        AchievementKind::Ft8Pathfinder => (
            (contacts
                .iter()
                .filter(|contact| contact.mode.eq_ignore_ascii_case("FT8"))
                .count() as u32)
                .min(25),
            25,
        ),
        AchievementKind::Ft4Pathfinder => (
            (contacts
                .iter()
                .filter(|contact| contact.mode.eq_ignore_ascii_case("FT4"))
                .count() as u32)
                .min(25),
            25,
        ),
        AchievementKind::CwOperator => (
            (contacts
                .iter()
                .filter(|contact| contact.mode.eq_ignore_ascii_case("CW"))
                .count() as u32)
                .min(10),
            10,
        ),
    }
}

pub(super) fn load_automation_achievement_definitions() -> Vec<AchievementDefinition> {
    let source = include_str!("../../../achievements.example.toml");
    qsonaut_automation::AchievementCatalog::from_toml(source)
        .map(|catalog| catalog.achievements)
        .unwrap_or_default()
}

impl QsonautGuiApp {
    pub(super) fn observe_automation_achievements(&mut self, event: &AutomationEvent) {
        let updates = self
            .automation_achievement_definitions
            .iter()
            .filter_map(|definition| {
                self.automation_achievement_evaluator
                    .observe(definition, event)
            })
            .filter(|update| update.unlocked)
            .collect::<Vec<_>>();
        for update in updates {
            let kind = match update.id.as_str() {
                "first-decode" => Some(AchievementKind::FirstDecode),
                "directed-call" => Some(AchievementKind::DirectedCall),
                "first-qso" => Some(AchievementKind::FirstQsoLogged),
                "ten-qsos" => Some(AchievementKind::TenQsosLogged),
                "fifty-qsos" => Some(AchievementKind::FiftyQsosLogged),
                "century-hunter" => Some(AchievementKind::CenturyHunter),
                "band-collector" => Some(AchievementKind::BandCollector),
                "grid-mapper" => Some(AchievementKind::GridMapper),
                "mode-cartographer" => Some(AchievementKind::ModeCartographer),
                "state-line" => Some(AchievementKind::StateLine),
                "qso-quarter" => Some(AchievementKind::QsoQuarter),
                "audio-alchemist" => Some(AchievementKind::AudioAlchemist),
                "dupe-shield" => Some(AchievementKind::DupeShield),
                "dx-chaser" => Some(AchievementKind::DXChaser),
                "early-bird" => Some(AchievementKind::EarlyBird),
                "night-owl" => Some(AchievementKind::NightOwl),
                "contest-operator" => Some(AchievementKind::ContestOperator),
                "signal-survivor" => Some(AchievementKind::SignalSurvivor),
                "ft8-pathfinder" => Some(AchievementKind::Ft8Pathfinder),
                "ft4-pathfinder" => Some(AchievementKind::Ft4Pathfinder),
                "cw-operator" => Some(AchievementKind::CwOperator),
                _ => None,
            };
            if let Some(kind) = kind {
                self.unlock_achievement(kind, update.title, update.detail);
            }
        }
    }

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
        if !self.hunter_alerts_enabled {
            return;
        }
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

        let id = custom_rule_id(title, &self.hunter_custom_rules);

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
            self.observe_automation_achievements(&AutomationEvent::new(
                EventKind::Decode,
                "app.decode_batch",
            ));
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

        let mut mode_counts = BTreeMap::<String, usize>::new();
        for contact in &self.qso_log.contacts {
            let mode = contact.mode.trim().to_ascii_uppercase();
            if !mode.is_empty() {
                *mode_counts.entry(mode).or_default() += 1;
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "Automation catalog: {} definitions",
                    self.automation_achievement_definitions.len()
                ))
                .small()
                .color(Color32::from_rgb(132, 228, 255)),
            );
            if !mode_counts.is_empty() {
                ui.separator();
                ui.label(RichText::new("Per-mode QSOs:").small().color(Color32::GRAY));
                for (mode, count) in mode_counts {
                    ui.label(RichText::new(format!("{mode} {count}")).small().monospace());
                }
            }
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
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Built-in achievements")
                    .strong()
                    .color(Color32::from_rgb(255, 201, 92)),
            );
            ui.checkbox(&mut self.hunter_alerts_enabled, "Alerts enabled");
            let acknowledged_label = if self.hunter_show_acknowledged {
                "Hide acknowledged"
            } else {
                "Show acknowledged"
            };
            if ui.button(acknowledged_label).clicked() {
                self.hunter_show_acknowledged = !self.hunter_show_acknowledged;
            }
        });
        let qso_count = self.qso_log.contacts.len() as u32;
        for achievement in AchievementKind::ALL {
            let (title, detail) = achievement.presentation();
            let (progress, target) = achievement_progress(
                achievement,
                qso_count,
                self.hunter_unique_heard.len() as u32,
                self.hunter_dupe_blocks,
                self.hunter_decode_bursts,
                &self.qso_log.contacts,
            );
            let unlocked = self.hunter_unlocked.contains(&achievement);
            let acknowledged = self.hunter_acknowledged.contains(&achievement);
            if unlocked && acknowledged && !self.hunter_show_acknowledged {
                continue;
            }
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(if unlocked { "🏆" } else { "🔒" }).color(
                        if unlocked {
                            Color32::from_rgb(255, 201, 92)
                        } else {
                            Color32::GRAY
                        },
                    ));
                    ui.label(RichText::new(title).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if unlocked
                            && !acknowledged
                            && ui
                                .small_button("Acknowledge")
                                .on_hover_text(
                                    "Hide this completed achievement from the default list",
                                )
                                .clicked()
                        {
                            self.hunter_acknowledged.insert(achievement);
                            self.profile_dirty = true;
                            self.persist_profile("Achievement acknowledged in");
                        }
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
        ui.label(
            RichText::new("Acknowledged achievements are hidden from the default list. Click Show acknowledged to restore them.")
                .small()
                .color(Color32::GRAY),
        );
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
                    self.observe_automation_achievements(&event);
                    let report = self.automation_host.dispatch(&event);

                    for approved in &report.approved {
                        let action_name = match &approved.action {
                            Action::Notify { .. } => "notify",
                            Action::SetCompose { .. } => "set_compose",
                            Action::SendExternal { .. } => "send_external",
                            Action::ServerSync => "server_sync",
                            Action::ServerSendMessage { .. } => "server_send_message",
                            Action::RadioCommand { .. } => "radio_command",
                            Action::ReadControl { .. } => "read_control",
                            Action::WriteControl { .. } => "write_control",
                            Action::ControlSequence { .. } => "control_sequence",
                            Action::RequestTransmit { .. } => "request_transmit",
                        };
                        info!(event = %event.source, action = action_name, "Automation action approved");
                        match &approved.action {
                            Action::Notify {
                                title,
                                body,
                                accent,
                            } => {
                                let accent = accent.as_deref().unwrap_or("default");
                                self.automation_status =
                                    format!("{title} — {body} (accent: {accent})");
                            }
                            Action::SetCompose { mode, message } => {
                                let normalized_mode = mode.trim().to_ascii_uppercase();
                                if normalized_mode == "FT8" {
                                    self.ft8_compose = message.clone();
                                    self.automation_status =
                                        "Automation prepared FT8 compose text".to_string();
                                } else {
                                    self.digital_compose = message.clone();
                                    self.automation_status = format!(
                                        "Automation prepared {} compose text",
                                        normalized_mode
                                    );
                                }
                            }
                            Action::SendExternal {
                                source,
                                target,
                                message,
                            } => {
                                self.automation_status =
                                    self.execute_automation_external_send(source, target, message);
                            }
                            Action::ServerSync => {
                                self.automation_status = if let Some(client) = &self.server_client {
                                    client.request_sync();
                                    "Requested QSONaut Server sync".to_string()
                                } else {
                                    "Server sync unavailable: no configured connection".to_string()
                                };
                            }
                            Action::ServerSendMessage { channel, message } => {
                                self.automation_status =
                                    if channel.trim().is_empty() || message.trim().is_empty() {
                                        "Server publish rejected: channel and message are required"
                                            .to_string()
                                    } else if channel.chars().count() > 80
                                        || message.chars().count() > 2_000
                                    {
                                        "Server publish rejected: message exceeds server limits"
                                            .to_string()
                                    } else if let Some(client) = &self.server_client {
                                        client.publish_channel_message(channel, message);
                                        format!("Published automation message to #{channel}")
                                    } else {
                                        "Server publish unavailable: no configured connection"
                                            .to_string()
                                    };
                            }
                            Action::RadioCommand { command, value } => {
                                self.automation_status =
                                    self.execute_automation_radio_command(command, value);
                            }
                            Action::ReadControl {
                                control,
                                result_key,
                            } => {
                                self.automation_status =
                                    self.execute_automation_control_read(control, result_key);
                            }
                            Action::WriteControl { control, value } => {
                                self.automation_status =
                                    self.execute_automation_control_write(control, value);
                            }
                            Action::ControlSequence { name, steps } => {
                                self.automation_status =
                                    self.execute_automation_control_sequence(name, steps);
                            }
                            Action::RequestTransmit { mode, message } => {
                                self.automation_status =
                                    self.execute_automation_transmit_request(mode, message);
                            }
                        }
                        let status = self.automation_status.to_ascii_lowercase();
                        let failed = ["rejected", "blocked", "unavailable", "failed", "error"]
                            .iter()
                            .any(|marker| status.contains(marker));
                        if failed {
                            warn!(event = %event.source, action = action_name, "Automation action failed");
                        } else {
                            info!(event = %event.source, action = action_name, "Automation action completed");
                        }
                    }

                    if !report.denied.is_empty() {
                        warn!(event = %event.source, denied = report.denied.len(), "Automation actions denied");
                        self.automation_status = format!(
                            "Automation denied {} action(s) (capability not granted/requested)",
                            report.denied.len()
                        );
                    }
                    if let Some(error) = report.errors.first() {
                        self.automation_status = format!("Automation component error: {error}");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(skipped)) => {
                    warn!(skipped, "Automation event stream lagged");
                    self.automation_status =
                        format!("Automation event stream lagged; skipped {skipped} event(s)");
                }
                Err(TryRecvError::Closed) => {
                    error!("Automation event stream closed");
                    self.automation_status =
                        "Automation event stream closed; restart required".to_string();
                    break;
                }
            }
        }
    }

    pub(super) fn pump_server_automation_events(&mut self) {
        let Some(client) = &self.server_client else {
            return;
        };
        for event in client.drain_automation_events() {
            info!(kind = %event.kind, field_count = event.fields.len(), "Server event received");
            if event.kind == "diagnostic_accepted" {
                let id = event
                    .fields
                    .get("id")
                    .map(String::as_str)
                    .unwrap_or_default();
                let short_id = id.get(..8).unwrap_or(id);
                self.profile_io_status = if short_id.is_empty() {
                    "Diagnostic snapshot accepted by QSONaut Server".to_string()
                } else {
                    format!("Diagnostic snapshot accepted by QSONaut Server · {short_id}")
                };
            } else if event.kind == "error" {
                warn!(kind = %event.kind, "Server reported request failure");
                if let Some(message) = event.fields.get("message") {
                    self.profile_io_status = format!("QSONaut Server rejected request: {message}");
                }
            }
            info!(kind = %event.kind, "Server event published to application event bus");
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
                if !native_radio_profile(&self.config.radio.backend, &self.config.radio.model)
                    .is_some_and(|profile| profile.supports_control(ControlId::Filter)) =>
            {
                "Rejected radio command: selected profile has no filter control".to_string()
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

    fn execute_automation_control_write(
        &mut self,
        control: &str,
        value: &qsonaut_automation::ControlValue,
    ) -> String {
        let Some(control_id) = automation_control_id(control) else {
            return format!("Rejected control write: unknown HAL control '{control}'");
        };
        let Some(value) = automation_control_value(value) else {
            return format!("Rejected control write: invalid value for '{control}'");
        };
        self.send_command(GuiCommand::SetControl(control_id, value));
        format!("Queued HAL control write for {control}")
    }

    pub(super) fn execute_automation_control_read(
        &mut self,
        control: &str,
        result_key: &str,
    ) -> String {
        let Some(control_id) = automation_control_id(control) else {
            return format!("Rejected control read: unknown HAL control '{control}'");
        };
        let Some(tx) = &self.command_tx else {
            return "Rejected control read: radio worker is unavailable".to_string();
        };
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        if tx
            .send(GuiCommand::ReadControl(control_id, ack_tx))
            .is_err()
        {
            return "Rejected control read: radio worker is unavailable".to_string();
        }
        match ack_rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(Ok(Some(value))) => {
                let value = automation_radio_value_text(&value);
                self.automation_host.set_variable(result_key, &value);
                let mut fields = BTreeMap::new();
                fields.insert("control".to_string(), control.trim().to_string());
                fields.insert("result_key".to_string(), result_key.trim().to_string());
                fields.insert("value".to_string(), value.clone());
                fields.insert("ok".to_string(), "true".to_string());
                self.app_events.publish(AppEvent::AutomationResult {
                    source: "gui.radio.control_read".to_string(),
                    fields,
                });
                format!("Read {control} into {result_key}: {value}")
            }
            Ok(Ok(None)) => format!("Read {control} into {result_key}: unavailable"),
            Ok(Err(error)) => format!("Control read rejected: {error}"),
            Err(_) => "Control read timed out: radio worker did not respond".to_string(),
        }
    }

    fn execute_automation_control_sequence(
        &mut self,
        name: &str,
        steps: &[qsonaut_automation::ControlStep],
    ) -> String {
        if steps.is_empty() {
            return format!("Rejected control sequence '{name}': no steps");
        }
        let Some(tx) = self.command_tx.clone() else {
            return format!("Rejected control sequence '{name}': radio worker unavailable");
        };
        let mut converted = Vec::with_capacity(steps.len());
        for step in steps {
            let Some(control_id) = automation_control_id(&step.control) else {
                return format!(
                    "Control sequence '{name}' stopped: unknown HAL control '{}'",
                    step.control
                );
            };
            let Some(value) = automation_control_value(&step.value) else {
                return format!(
                    "Control sequence '{name}' stopped: invalid value for '{}'",
                    step.control
                );
            };
            converted.push((control_id, value, step.wait_ms.min(60_000)));
        }
        std::thread::spawn(move || {
            for (control_id, value, wait_ms) in converted {
                if tx.send(GuiCommand::SetControl(control_id, value)).is_err() {
                    break;
                }
                if wait_ms > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                }
            }
        });
        format!("Queued control sequence '{name}' ({} steps)", steps.len())
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

pub(super) const AUTOMATION_CONTROL_CATALOG: &[(&str, &str, &str)] = &[
    ("af_gain", "Audio gain", "u8"),
    ("rf_gain", "RF gain", "u8"),
    ("squelch", "Squelch", "u8"),
    ("rf_power", "RF power", "u8"),
    ("preamp", "Preamp", "bool"),
    ("attenuator", "Attenuator", "bool"),
    ("noise_blanker", "Noise blanker", "bool"),
    ("noise_reduction", "Noise reduction", "bool"),
    ("noise_reduction_level", "Noise reduction level", "u8"),
    ("ip_plus", "IP+", "bool"),
    ("notch", "Notch", "bool"),
    ("manual_notch", "Manual notch", "bool"),
    ("manual_notch_position", "Manual notch position", "i32"),
    ("data_mode", "Data mode", "bool"),
    ("filter", "Filter", "u8"),
    ("tuning_step", "Tuning step", "u64"),
    ("agc", "AGC", "u8"),
    ("rit", "RIT", "i32"),
    ("xit", "XIT", "i32"),
    ("split", "Split", "bool"),
    ("tuner", "Tuner", "bool"),
    ("raw_civ", "Raw CI-V", "raw_hex"),
    ("vfo", "VFO", "vfo"),
    ("main_sub", "Main/Sub", "u8"),
    ("external_preamp", "External preamp", "u8"),
    ("antenna", "Antenna", "u8"),
    ("mic_gain", "Mic gain", "u8"),
    ("monitor_level", "Monitor level", "u8"),
    ("speech_processor", "Speech processor", "bool"),
    ("speech_processor_level", "Speech processor level", "u8"),
    ("if_shift", "IF shift", "i32"),
    ("vox", "VOX", "bool"),
    ("vox_gain", "VOX gain", "u8"),
    ("vox_delay", "VOX delay", "u8"),
    ("break_in", "Break-in", "bool"),
    ("lock", "Panel lock", "bool"),
    ("noise_blanker_level", "Noise blanker level", "u8"),
];

fn automation_control_id(name: &str) -> Option<ControlId> {
    match name.trim().to_ascii_lowercase().as_str() {
        "af_gain" => Some(ControlId::AfGain),
        "rf_gain" => Some(ControlId::RfGain),
        "squelch" => Some(ControlId::Squelch),
        "rf_power" => Some(ControlId::RfPower),
        "preamp" => Some(ControlId::Preamp),
        "attenuator" => Some(ControlId::Attenuator),
        "noise_blanker" => Some(ControlId::NoiseBlanker),
        "noise_reduction" => Some(ControlId::NoiseReduction),
        "noise_reduction_level" => Some(ControlId::NoiseReductionLevel),
        "ip_plus" => Some(ControlId::IpPlus),
        "notch" => Some(ControlId::Notch),
        "manual_notch" => Some(ControlId::ManualNotch),
        "manual_notch_position" => Some(ControlId::ManualNotchPosition),
        "data_mode" => Some(ControlId::DataMode),
        "filter" => Some(ControlId::Filter),
        "tuning_step" => Some(ControlId::TuningStep),
        "agc" => Some(ControlId::Agc),
        "rit" => Some(ControlId::Rit),
        "xit" => Some(ControlId::Xit),
        "split" => Some(ControlId::Split),
        "tuner" => Some(ControlId::Tuner),
        "raw_civ" => Some(ControlId::RawCiV),
        "vfo" => Some(ControlId::Vfo),
        "main_sub" => Some(ControlId::MainSub),
        "external_preamp" => Some(ControlId::ExternalPreamp),
        "antenna" => Some(ControlId::Antenna),
        "mic_gain" => Some(ControlId::MicGain),
        "monitor_level" => Some(ControlId::MonitorLevel),
        "speech_processor" => Some(ControlId::SpeechProcessor),
        "speech_processor_level" => Some(ControlId::SpeechProcessorLevel),
        "if_shift" => Some(ControlId::IfShift),
        "vox" => Some(ControlId::Vox),
        "vox_gain" => Some(ControlId::VoxGain),
        "vox_delay" => Some(ControlId::VoxDelay),
        "break_in" => Some(ControlId::BreakIn),
        "lock" => Some(ControlId::Lock),
        "noise_blanker_level" => Some(ControlId::NoiseBlankerLevel),
        _ => None,
    }
}

fn automation_control_value(
    value: &qsonaut_automation::ControlValue,
) -> Option<qsonaut_radio::ControlValue> {
    match value {
        qsonaut_automation::ControlValue::Bool(value) => {
            Some(qsonaut_radio::ControlValue::Bool(*value))
        }
        qsonaut_automation::ControlValue::U8(value) => {
            Some(qsonaut_radio::ControlValue::U8(*value))
        }
        qsonaut_automation::ControlValue::I32(value) => {
            Some(qsonaut_radio::ControlValue::I32(*value))
        }
        qsonaut_automation::ControlValue::U64(value) => {
            Some(qsonaut_radio::ControlValue::U64(*value))
        }
        qsonaut_automation::ControlValue::Text(value) => {
            Some(qsonaut_radio::ControlValue::Text(value.clone()))
        }
        qsonaut_automation::ControlValue::RawHex(value) => {
            let compact = value.replace([' ', ':', '-'], "");
            if compact.is_empty() || compact.len() % 2 != 0 {
                return None;
            }
            let bytes = (0..compact.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&compact[index..index + 2], 16))
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            Some(qsonaut_radio::ControlValue::Raw(bytes))
        }
    }
}

fn automation_radio_value_text(value: &qsonaut_radio::ControlValue) -> String {
    match value {
        qsonaut_radio::ControlValue::Bool(value) => value.to_string(),
        qsonaut_radio::ControlValue::U8(value)
        | qsonaut_radio::ControlValue::Vfo(value)
        | qsonaut_radio::ControlValue::Receiver(value) => value.to_string(),
        qsonaut_radio::ControlValue::I32(value) => value.to_string(),
        qsonaut_radio::ControlValue::U64(value) => value.to_string(),
        qsonaut_radio::ControlValue::Mode(value) => format!("{value:?}"),
        qsonaut_radio::ControlValue::Text(value) => value.clone(),
        qsonaut_radio::ControlValue::Raw(value) => value
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{
        achievement_progress, custom_rule_id, AchievementKind, CustomAchievementRule, HunterMetric,
        QsoRecord,
    };
    use std::sync::{Arc, Mutex};

    fn headless_app() -> QsonautGuiApp {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        QsonautGuiApp::new_with_context(
            AppConfig::default(),
            false,
            false,
            &egui::Context::default(),
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        )
    }

    fn existing(id: &str) -> CustomAchievementRule {
        CustomAchievementRule {
            id: id.to_string(),
            title: String::new(),
            detail: String::new(),
            metric: HunterMetric::UniqueHeard,
            threshold: 1,
            enabled: true,
            unlocked: false,
        }
    }

    #[test]
    fn creates_stable_custom_rule_ids_and_disambiguates_duplicates() {
        assert_eq!(custom_rule_id("  My Grid!  ", &[]), "my-grid");
        assert_eq!(custom_rule_id("!!!", &[]), "custom-1");
        assert_eq!(
            custom_rule_id("My Grid!", &[existing("my-grid")]),
            "my-grid-2"
        );
    }

    #[test]
    fn every_builtin_achievement_has_presentation_text() {
        assert_eq!(AchievementKind::ALL.len(), 22);
        for kind in AchievementKind::ALL {
            let (title, detail) = kind.presentation();
            assert!(!title.is_empty());
            assert!(!detail.is_empty());
        }
    }

    #[test]
    fn achievement_progress_normalizes_counts_and_distinct_contact_dimensions() {
        let mut contacts = Vec::new();
        for (band, grid, mode) in [
            ("20m", "FN42", "FT8"),
            ("40m", "EN50", "FT4"),
            ("2m", "FN42", "CW"),
            ("", "", ""),
        ] {
            let mut contact = QsoRecord::new("K1ABC", mode, band, 14_074_000, 0, 1);
            contact.grid = grid.to_string();
            contacts.push(contact);
        }
        assert_eq!(
            achievement_progress(AchievementKind::FirstDecode, 80, 120, 20, 9, &contacts),
            (1, 1)
        );
        assert_eq!(
            achievement_progress(AchievementKind::DirectedCall, 80, 120, 20, 9, &contacts),
            (0, 1)
        );
        assert_eq!(
            achievement_progress(AchievementKind::TenQsosLogged, 80, 120, 20, 9, &contacts),
            (10, 10)
        );
        assert_eq!(
            achievement_progress(AchievementKind::FiftyQsosLogged, 80, 120, 20, 9, &contacts),
            (50, 50)
        );
        assert_eq!(
            achievement_progress(AchievementKind::DupeShield, 80, 120, 20, 9, &contacts),
            (10, 10)
        );
        assert_eq!(
            achievement_progress(AchievementKind::CenturyHunter, 80, 120, 20, 9, &contacts),
            (100, 100)
        );
        assert_eq!(
            achievement_progress(AchievementKind::BandCollector, 80, 120, 20, 9, &contacts),
            (3, 5)
        );
        assert_eq!(
            achievement_progress(AchievementKind::GridMapper, 80, 120, 20, 9, &contacts),
            (2, 25)
        );
        assert_eq!(
            achievement_progress(AchievementKind::ModeExplorer, 80, 120, 20, 9, &contacts),
            (3, 3)
        );
        assert_eq!(
            achievement_progress(
                AchievementKind::AudioAlchemist,
                80,
                120,
                20,
                1_200,
                &contacts
            ),
            (1_000, 1_000)
        );
        assert_eq!(
            achievement_progress(AchievementKind::QsoQuarter, 80, 120, 20, 9, &contacts),
            (25, 25)
        );
        assert_eq!(
            achievement_progress(AchievementKind::ModeCartographer, 80, 120, 20, 9, &contacts),
            (3, 3)
        );
        for kind in [
            AchievementKind::DXChaser,
            AchievementKind::EarlyBird,
            AchievementKind::NightOwl,
            AchievementKind::ContestOperator,
            AchievementKind::SignalSurvivor,
        ] {
            assert_eq!(
                achievement_progress(kind, 80, 120, 20, 9, &contacts),
                (0, 1)
            );
        }
    }

    #[test]
    fn hunter_rule_evaluation_unlocks_enabled_thresholds_and_ignores_disabled_rules() {
        let mut app = headless_app();
        app.hunter_unique_heard.insert("K1ABC".to_string());
        app.hunter_custom_rules = vec![
            CustomAchievementRule {
                id: "heard".to_string(),
                title: "Heard one".to_string(),
                detail: "A station was heard".to_string(),
                metric: HunterMetric::UniqueHeard,
                threshold: 1,
                enabled: true,
                unlocked: false,
            },
            CustomAchievementRule {
                id: "disabled".to_string(),
                title: "Disabled".to_string(),
                detail: "Must remain locked".to_string(),
                metric: HunterMetric::DecodeBursts,
                threshold: 1,
                enabled: false,
                unlocked: false,
            },
        ];
        app.evaluate_custom_hunter_rules();
        assert!(app.hunter_custom_rules[0].unlocked);
        assert!(!app.hunter_custom_rules[1].unlocked);
        assert_eq!(app.hunter_feed.len(), 1);
    }

    #[test]
    fn automation_radio_commands_reject_unsafe_and_invalid_values() {
        let mut app = headless_app();
        app.config.radio.model = "unknown-model".to_string();
        assert!(app
            .execute_automation_radio_command("tune_delta_hz", "-25")
            .contains("-25 Hz"));
        assert!(app
            .execute_automation_radio_command("tune_delta_hz", "bad")
            .contains("invalid tune delta"));
        assert!(app
            .execute_automation_radio_command("tune_workspace_band_hz", "0")
            .contains("must be > 0"));
        assert!(app
            .execute_automation_radio_command("tune_workspace_band_hz", "bad")
            .contains("invalid frequency"));
        assert!(app
            .execute_automation_radio_command("set_filter", "bad")
            .contains("no filter control"));
        assert!(app
            .execute_automation_radio_command("set_ptt", "1")
            .contains("not allowed"));
        assert!(app
            .execute_automation_radio_command("unknown", "1")
            .contains("unsupported command"));
        assert!(app
            .execute_automation_radio_command("cycle_mode", "")
            .contains("mode cycle"));
    }

    #[test]
    fn automation_control_catalog_contains_only_resolvable_controls() {
        for &(name, _, _) in AUTOMATION_CONTROL_CATALOG {
            assert!(
                automation_control_id(name).is_some(),
                "catalog control {name} has no HAL resolver"
            );
        }
    }

    #[test]
    fn external_automation_send_validates_transport_and_bounds_outbox() {
        let mut app = headless_app();
        app.automation_external_transports.clear();
        assert!(app
            .execute_automation_external_send("", "target", "message")
            .contains("required"));
        assert!(app
            .execute_automation_external_send("irc:station", "target", "message")
            .contains("not configured"));
        app.automation_external_transports.insert("irc".to_string());
        for index in 0..40 {
            let result = app.execute_automation_external_send(
                "irc:station",
                "target",
                &format!("message-{index}"),
            );
            assert!(result.contains("Queued external send"));
        }
        assert_eq!(app.automation_external_outbox.len(), 32);
    }
}
