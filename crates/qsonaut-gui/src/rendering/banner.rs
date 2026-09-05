use super::super::*;
use crate::ui_widgets::{operating_mode_button, OperatingModeIcon};

// Visible roadmap entries only. These are deliberately not WorkspaceMode
// variants until an implementation and a legally usable protocol boundary
// exist.
const FUTURE_TEXT_MODES: &[(&str, &str, OperatingModeIcon)] = &[(
    "JS8Call",
    "Future text modem placeholder; protocol support is not enabled",
    OperatingModeIcon::Text,
)];
const FUTURE_VOICE_MODES: &[(&str, &str, OperatingModeIcon)] = &[
    (
        "VaraAC",
        "Future voice modem placeholder; protocol support is not enabled",
        OperatingModeIcon::VaraAc,
    ),
    (
        "RADE",
        "Future voice modem placeholder; protocol support is not enabled",
        OperatingModeIcon::Rade,
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
        let tuning_step_hz = match snapshot.tuning_step.unwrap_or(5) {
            0 => 1,
            1 => 5,
            2 => 10,
            3 => 50,
            4 => 100,
            _ => 1_000,
        };
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                ui.spacing_mut().button_padding.x = 4.0;
                ui.label(RichText::new("Radio").strong());
                if ui
                    .small_button(format!("-{tuning_step_hz} Hz"))
                    .on_hover_text(format!("Tune the radio down by {tuning_step_hz} Hz"))
                    .clicked()
                {
                    self.send_command(GuiCommand::TuneDelta(-(tuning_step_hz as i64)));
                }
                if ui
                    .small_button(format!("+{tuning_step_hz} Hz"))
                    .on_hover_text(format!("Tune the radio up by {tuning_step_hz} Hz"))
                    .clicked()
                {
                    self.send_command(GuiCommand::TuneDelta(tuning_step_hz as i64));
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
            let mut draw_mode =
                |ui: &mut egui::Ui, icon: OperatingModeIcon, mode: WorkspaceMode| {
                    let response = operating_mode_button(
                        ui,
                        self.workspace_mode == mode,
                        mode.label(),
                        icon,
                        true,
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
                draw_mode(ui, OperatingModeIcon::Digital, mode);
            }
            draw_mode(ui, OperatingModeIcon::Wspr, WorkspaceMode::Wspr);
            draw_mode(ui, OperatingModeIcon::Cw, WorkspaceMode::Cw);
            draw_mode(ui, OperatingModeIcon::Sstv, WorkspaceMode::Sstv);
            let response = operating_mode_button(
                ui,
                self.workspace_mode == WorkspaceMode::Voice,
                "Voice",
                OperatingModeIcon::Voice,
                true,
            )
            .on_hover_text("Switch workspace to Voice");
            if response.clicked() {
                self.workspace_mode = WorkspaceMode::Voice;
                self.profile_dirty = true;
                self.persist_profile("Mode saved to");
                if let Some(frequency_hz) = workspace_frequency_for_current_band(
                    WorkspaceMode::Voice,
                    snapshot.frequency_hz,
                ) {
                    self.send_command(GuiCommand::ApplyWorkspace {
                        mode: WorkspaceMode::Voice,
                        frequency_hz,
                    });
                }
            }

            let mode = WorkspaceMode::Msk144;
            let enabled = !mode.is_uhf();
            let response = operating_mode_button(
                ui,
                self.workspace_mode == mode,
                mode.label(),
                OperatingModeIcon::Msk144,
                enabled,
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

            for (label, tooltip, icon) in FUTURE_TEXT_MODES.iter().copied() {
                operating_mode_button(ui, false, label, icon, false)
                    .on_disabled_hover_text(tooltip);
            }
            for (label, tooltip, icon) in FUTURE_VOICE_MODES.iter().copied() {
                operating_mode_button(ui, false, label, icon, false)
                    .on_disabled_hover_text(tooltip);
            }
        });
    }
}
