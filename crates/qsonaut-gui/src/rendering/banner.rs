use super::super::*;

// Visible roadmap entries only. These are deliberately not WorkspaceMode
// variants until an implementation and a legally usable protocol boundary
// exist.
const FUTURE_TEXT_MODES: &[(&str, &str)] = &[(
    "⌨ JS8Call",
    "Future text modem placeholder; protocol support is not enabled",
)];
const FUTURE_VOICE_MODES: &[(&str, &str)] = &[
    (
        "🎙 VaraAC",
        "Future voice modem placeholder; protocol support is not enabled",
    ),
    (
        "🎙 RADE",
        "Future voice modem placeholder; protocol support is not enabled",
    ),
];

impl QsonautGuiApp {
    pub(crate) fn draw_header_branding(&mut self, ui: &mut egui::Ui) {
        let spin_angle = self.logo_spin_until.map_or(0.0, |until| {
            let remaining = until.saturating_duration_since(Instant::now());
            (1.0 - remaining.as_secs_f32() / 0.7).clamp(0.0, 1.0) * std::f32::consts::TAU
        });
        let logo = egui::Image::new((self.brand_icon.id(), egui::vec2(56.0, 56.0)))
            .corner_radius(8.0)
            .rotate(spin_angle, egui::Vec2::splat(0.5))
            .sense(egui::Sense::click());
        let logo_response = ui
            .add(logo)
            .on_hover_text("QSONaut mission control — click for the application animation");
        if logo_response.clicked() {
            self.handle_logo_click();
        }
        if self
            .logo_spin_until
            .is_some_and(|until| Instant::now() < until)
        {
            ui.ctx().request_repaint();
        } else {
            self.logo_spin_until = None;
        }
        ui.vertical(|ui| {
            ui.label(
                RichText::new("QSONaut")
                    .strong()
                    .size(32.0)
                    .color(Color32::from_rgb(109, 224, 255)),
            );
            ui.label(
                RichText::new("AMATEUR RADIO MISSION CONTROL")
                    .strong()
                    .size(10.0)
                    .color(Color32::from_rgb(255, 137, 108)),
            );
        });
        self.draw_activity_selector(ui);
    }
}

impl QsonautGuiApp {
    pub(crate) fn draw_banner_radio_controls(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let supports_levels = snapshot.supported_controls.contains(&ControlId::AfGain);
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                ui.spacing_mut().button_padding.x = 4.0;
                ui.label(RichText::new("Radio").strong());
                if ui
                    .small_button("-1 kHz")
                    .on_hover_text("Tune the radio down by 1 kHz")
                    .clicked()
                {
                    self.send_command(GuiCommand::TuneDelta(-1_000));
                }
                if ui
                    .small_button("+1 kHz")
                    .on_hover_text("Tune the radio up by 1 kHz")
                    .clicked()
                {
                    self.send_command(GuiCommand::TuneDelta(1_000));
                }
                if ui
                    .add_enabled(supports_levels, egui::Button::new("AF-").small())
                    .on_disabled_hover_text("Audio receive gain is not supported by this radio")
                    .on_hover_text("Decrease audio receive gain")
                    .clicked()
                {
                    self.send_command(GuiCommand::AfGainDelta(-5));
                }
                if ui
                    .add_enabled(supports_levels, egui::Button::new("AF+").small())
                    .on_disabled_hover_text("Audio receive gain is not supported by this radio")
                    .on_hover_text("Increase audio receive gain")
                    .clicked()
                {
                    self.send_command(GuiCommand::AfGainDelta(5));
                }
            });
        });
    }

    pub(crate) fn draw_banner_op_modes(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let mut draw_mode = |ui: &mut egui::Ui, icon: &str, mode: WorkspaceMode| {
                let response = ui
                    .add(
                        egui::Button::selectable(
                            self.workspace_mode == mode,
                            RichText::new(format!("{icon} {}", mode.label())).size(12.0),
                        )
                        .small(),
                    )
                    .on_hover_text(format!("Switch workspace to {}", mode.label()));
                if response.clicked() {
                    self.workspace_mode = mode;
                    self.profile_dirty = true;
                    self.persist_profile("Mode saved to");
                    if let Some(frequency_hz) =
                        workspace_frequency_for_current_band(mode, snapshot.frequency_hz)
                    {
                        self.send_command(GuiCommand::ApplyWorkspace { mode, frequency_hz });
                    }
                }
            };
            for mode in [
                WorkspaceMode::Ft8,
                WorkspaceMode::Ft4,
                WorkspaceMode::Fst4,
                WorkspaceMode::Jt9,
                WorkspaceMode::Jt65,
                WorkspaceMode::Q65,
            ] {
                draw_mode(ui, "⌨", mode);
            }
            draw_mode(ui, "📡", WorkspaceMode::Wspr);
            draw_mode(ui, "⌨", WorkspaceMode::Cw);
            draw_mode(ui, "🎙", WorkspaceMode::Voice);
            draw_mode(ui, "🖼", WorkspaceMode::Sstv);

            let mode = WorkspaceMode::Msk144;
            let enabled = !mode.is_uhf();
            let response = ui
                .add_enabled(
                    enabled,
                    egui::Button::selectable(
                        self.workspace_mode == mode,
                        RichText::new(format!("📶 {}", mode.label())).size(12.0),
                    )
                    .small(),
                )
                .on_hover_text(if enabled {
                    format!("Switch workspace to {}", mode.label())
                } else {
                    format!(
                        "{} is disabled without a configured UHF radio",
                        mode.label()
                    )
                });
            if response.clicked() && enabled {
                self.workspace_mode = mode;
                self.profile_dirty = true;
                self.persist_profile("Mode saved to");
                if let Some(frequency_hz) =
                    workspace_frequency_for_current_band(mode, snapshot.frequency_hz)
                {
                    self.send_command(GuiCommand::ApplyWorkspace { mode, frequency_hz });
                }
            }

            for (label, tooltip) in FUTURE_TEXT_MODES.iter().chain(FUTURE_VOICE_MODES.iter()) {
                ui.add_enabled(
                    false,
                    egui::Button::selectable(false, RichText::new(*label).size(12.0)).small(),
                )
                .on_disabled_hover_text(*tooltip);
            }
        });
    }
}
