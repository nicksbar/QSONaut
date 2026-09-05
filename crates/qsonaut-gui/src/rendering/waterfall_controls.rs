use super::super::*;

#[derive(Clone, Copy)]
enum WaterfallIcon {
    Audio,
    Radio,
    Disabled,
}

fn draw_waterfall_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: WaterfallIcon,
    color: Color32,
) {
    let center = rect.center();
    let stroke = egui::Stroke::new(1.4, color);
    match icon {
        WaterfallIcon::Audio => {
            for (index, offset) in [-4.0_f32, 0.0, 4.0].into_iter().enumerate() {
                let y = rect.top() + 4.0 + offset + index as f32;
                painter.line(
                    vec![
                        egui::pos2(rect.left() + 1.0, y),
                        egui::pos2(center.x - 5.0, y - 1.5),
                        egui::pos2(center.x - 1.0, y + 1.5),
                        egui::pos2(center.x + 3.0, y - 2.0),
                        egui::pos2(rect.right() - 1.0, y),
                    ],
                    stroke,
                );
            }
        }
        WaterfallIcon::Radio => {
            painter.circle_stroke(center, 5.0, stroke);
            painter.circle_filled(center, 1.5, color);
            painter.line_segment(
                [
                    egui::pos2(center.x - 7.0, center.y + 6.0),
                    egui::pos2(center.x + 7.0, center.y - 6.0),
                ],
                stroke,
            );
        }
        WaterfallIcon::Disabled => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(10.0, 10.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
}

fn waterfall_icon_button(ui: &mut egui::Ui, icon: WaterfallIcon, color: Color32) -> egui::Response {
    let response = ui.add(egui::Button::new("").min_size(egui::vec2(28.0, 24.0)));
    draw_waterfall_icon(ui.painter(), response.rect.shrink(4.0), icon, color);
    response
}

impl QsonautGuiApp {
    pub(crate) fn draw_waterfall_buttons(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let audio_button = waterfall_icon_button(ui, WaterfallIcon::Audio, Color32::LIGHT_BLUE)
            .on_hover_text("Audio waterfall controls");
        egui::Popup::menu(&audio_button)
            // Keep the drawer alive while nested combo boxes and sliders are
            // interacting with it. The banner redraws it every frame.
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(250.0);
                ui.label(RichText::new("AUDIO WATERFALL").strong());
                ui.label(RichText::new("Live audio spectrum display").small());
                ui.separator();
                ui.label(RichText::new("Theme").strong());
                ui.horizontal_wrapped(|ui| {
                    for theme in [
                        WaterfallTheme::RadioBlue,
                        WaterfallTheme::Inferno,
                        WaterfallTheme::Phosphor,
                        WaterfallTheme::Monochrome,
                    ] {
                        if ui
                            .selectable_label(self.waterfall_theme == theme, theme.label())
                            .clicked()
                        {
                            self.waterfall_theme = theme;
                            self.profile_dirty = true;
                            self.persist_profile("Waterfall theme saved to");
                        }
                    }
                });
                ui.label(RichText::new("Audio display speed").strong());
                {
                    let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
                    let selected = if tuning.audio_auto_visual {
                        "Auto"
                    } else {
                        tuning.audio_waterfall_speed.label()
                    };
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .selectable_label(tuning.audio_auto_visual, selected)
                            .clicked()
                        {
                            tuning.audio_auto_visual = true;
                        }
                        for speed in [
                            WaterfallSpeed::Fast,
                            WaterfallSpeed::Mid,
                            WaterfallSpeed::Slow,
                        ] {
                            let selected =
                                !tuning.audio_auto_visual && tuning.audio_waterfall_speed == speed;
                            if ui.selectable_label(selected, speed.label()).clicked() {
                                tuning.audio_auto_visual = false;
                                tuning.audio_waterfall_speed = speed;
                            }
                        }
                    });
                }
            });

        // The active driver is authoritative here. A configured radio may be
        // temporarily disabled while its negotiated metadata is still
        // available, and scope support should not disappear because of that
        // unrelated profile flag.
        let radio_scope_available = self.driver_metadata.scope.is_some();
        if radio_scope_available {
            let radio_button =
                waterfall_icon_button(ui, WaterfallIcon::Radio, Color32::from_rgb(180, 220, 255))
                    .on_hover_text("Native CI-V waterfall controls");
            egui::Popup::menu(&radio_button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(280.0);
                    ui.label(RichText::new("RADIO WATERFALL").strong());
                    ui.label(RichText::new("Native scope stream controls").small());
                    ui.separator();
                    if ui
                        .checkbox(&mut self.civ_spectrum_on, "Enable radio waterfall")
                        .changed()
                    {
                        self.profile_dirty = true;
                        self.persist_profile("Radio waterfall setting saved to");
                    }
                    ui.label(RichText::new("Radio waterfall theme").strong());
                    ui.horizontal_wrapped(|ui| {
                        for theme in [
                            WaterfallTheme::RadioBlue,
                            WaterfallTheme::Inferno,
                            WaterfallTheme::Phosphor,
                            WaterfallTheme::Monochrome,
                        ] {
                            if ui
                                .selectable_label(
                                    self.radio_waterfall_theme == theme,
                                    theme.label(),
                                )
                                .clicked()
                            {
                                self.radio_waterfall_theme = theme;
                                self.profile_dirty = true;
                                self.persist_profile("Radio waterfall theme saved to");
                            }
                        }
                    });
                    ui.label(RichText::new("Native sweep speed").strong());
                    let mut visual_changed = false;
                    {
                        let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
                        ui.horizontal_wrapped(|ui| {
                            ui.label(if tuning.radio_auto_visual {
                                "Auto (mode-driven)"
                            } else {
                                "Native speed"
                            });
                            for speed in [
                                WaterfallSpeed::Fast,
                                WaterfallSpeed::Mid,
                                WaterfallSpeed::Slow,
                            ] {
                                let selected = !tuning.radio_auto_visual
                                    && tuning.radio_waterfall_speed == speed;
                                if ui.selectable_label(selected, speed.label()).clicked() {
                                    tuning.radio_auto_visual = false;
                                    tuning.radio_waterfall_speed = speed;
                                    visual_changed = true;
                                }
                            }
                        });
                    }
                    if visual_changed {
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .radio_scope_settings_dirty = true;
                    }
                    let mut scope_changed = false;
                    ui.horizontal(|ui| {
                        scope_changed |= ui
                            .selectable_value(
                                &mut self.radio_scope_view,
                                RadioScopeView::Narrow,
                                "Narrow",
                            )
                            .changed();
                        scope_changed |= ui
                            .selectable_value(
                                &mut self.radio_scope_view,
                                RadioScopeView::Overview,
                                "Overview",
                            )
                            .changed();
                    });
                    scope_changed |= ui
                        .checkbox(&mut self.radio_scope_vbw_wide, "Wide VBW")
                        .changed();
                    ui.checkbox(&mut self.radio_scope_lock_if_to_filter, "Match span to FIL");
                    if self.radio_scope_lock_if_to_filter {
                        self.radio_scope_span_code = scope_span_for_filter_with_options(
                            &snapshot.mode,
                            self.driver_metadata.filter_bandwidth_hz(
                                &snapshot.mode,
                                snapshot.filter.unwrap_or_default(),
                            ),
                            self.driver_metadata
                                .scope
                                .map(|metadata| metadata.span_options_hz)
                                .unwrap_or(&[]),
                        );
                        ui.label(format!(
                            "Automatic span: {}",
                            scope_span_label_for(
                                self.driver_metadata.scope,
                                self.radio_scope_span_code
                            )
                        ));
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            let spans = self
                                .driver_metadata
                                .scope
                                .map(|metadata| metadata.span_options_hz)
                                .unwrap_or(&[]);
                            for (code, span_hz) in spans.iter().copied().enumerate() {
                                let code = code as u8;
                                let label = format!("±{} kHz", span_hz / 1_000);
                                if ui
                                    .selectable_label(self.radio_scope_span_code == code, label)
                                    .clicked()
                                {
                                    self.radio_scope_span_code = code;
                                    scope_changed = true;
                                }
                            }
                        });
                    }
                    scope_changed |= ui.checkbox(&mut self.radio_scope_hold, "Hold").changed();
                    scope_changed |= ui
                        .add(
                            egui::Slider::new(
                                &mut self.radio_scope_reference_tenths_db,
                                -200..=200,
                            )
                            .step_by(5.0)
                            .custom_formatter(|value, _| format!("{:.1} dB", value / 10.0))
                            .text("Reference"),
                        )
                        .changed();
                    let metadata = self.driver_metadata.scope;
                    if let Some(metadata) = metadata {
                        ui.separator();
                        ui.label(RichText::new("Driver scope options").strong());
                        ui.horizontal_wrapped(|ui| {
                            if !metadata.center_type_options.is_empty() {
                                ui.label("Center");
                                for value in metadata.center_type_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.center_type,
                                            Some(*value),
                                            format!("{value:?}"),
                                        )
                                        .changed();
                                }
                            }
                            if !metadata.tx_display_options.is_empty() {
                                ui.label("TX");
                                for value in metadata.tx_display_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.tx_display,
                                            Some(*value),
                                            if *value { "On" } else { "Off" },
                                        )
                                        .changed();
                                }
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if !metadata.max_hold_options.is_empty() {
                                ui.label("Max hold");
                                for value in metadata.max_hold_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.max_hold,
                                            Some(*value),
                                            format!("{value:?}"),
                                        )
                                        .changed();
                                }
                            }
                            if !metadata.marker_position_options.is_empty() {
                                ui.label("Marker");
                                for value in metadata.marker_position_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.marker_position,
                                            Some(*value),
                                            format!("{value:?}"),
                                        )
                                        .changed();
                                }
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if !metadata.averaging_options.is_empty() {
                                ui.label("Average");
                                for value in metadata.averaging_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.averaging,
                                            Some(*value),
                                            if *value == 0 {
                                                "Off".to_string()
                                            } else {
                                                format!("{value} sweeps")
                                            },
                                        )
                                        .changed();
                                }
                            }
                            if !metadata.waveform_type_options.is_empty() {
                                ui.label("Waveform");
                                for value in metadata.waveform_type_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.waveform_type,
                                            Some(*value),
                                            format!("{value:?}"),
                                        )
                                        .changed();
                                }
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if !metadata.waterfall_display_options.is_empty() {
                                ui.label("Display");
                                for value in metadata.waterfall_display_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.waterfall_display,
                                            Some(*value),
                                            if *value { "On" } else { "Off" },
                                        )
                                        .changed();
                                }
                            }
                            if !metadata.waterfall_size_options.is_empty() {
                                ui.label("Size");
                                for value in metadata.waterfall_size_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.waterfall_size,
                                            Some(*value),
                                            format!("{value}"),
                                        )
                                        .changed();
                                }
                            }
                            if !metadata.waterfall_peak_level_options.is_empty() {
                                ui.label("Peak");
                                for value in metadata.waterfall_peak_level_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.waterfall_peak_level,
                                            Some(*value),
                                            format!("{value}"),
                                        )
                                        .changed();
                                }
                            }
                        });
                        if !metadata.marker_auto_hide_options.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Marker auto-hide");
                                for value in metadata.marker_auto_hide_options {
                                    scope_changed |= ui
                                        .selectable_value(
                                            &mut self.radio_scope_advanced.marker_auto_hide,
                                            Some(*value),
                                            if *value { "On" } else { "Off" },
                                        )
                                        .changed();
                                }
                            });
                        }
                    }
                    if self
                        .driver_metadata
                        .scope
                        .is_some_and(|metadata| metadata.supports_waveform_colors)
                    {
                        ui.horizontal_wrapped(|ui| {
                            scope_changed |= edit_scope_color(
                                ui,
                                "Current",
                                &mut self.radio_scope_advanced.waveform_color_current,
                            );
                            scope_changed |= edit_scope_color(
                                ui,
                                "Line",
                                &mut self.radio_scope_advanced.waveform_color_line,
                            );
                            scope_changed |= edit_scope_color(
                                ui,
                                "Max hold",
                                &mut self.radio_scope_advanced.waveform_color_max_hold,
                            );
                        });
                    }
                    if scope_changed {
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .radio_scope_settings_dirty = true;
                    }
                });
        }

        let disabled_button = ui.add_enabled(
            false,
            egui::Button::new("").min_size(egui::vec2(28.0, 24.0)),
        );
        draw_waterfall_icon(
            ui.painter(),
            disabled_button.rect.shrink(4.0),
            WaterfallIcon::Disabled,
            Color32::from_gray(145),
        );
        disabled_button
        .on_disabled_hover_text(
            "IQ/SDR waterfall support is not enabled yet — development needs a radio that offers a supported IQ/SDR stream 😢",
        );
    }
}

fn edit_scope_color(ui: &mut egui::Ui, label: &str, color: &mut Option<ScopeColor>) -> bool {
    let mut rgba = color
        .map(|color| Color32::from_rgb(color.red, color.green, color.blue))
        .unwrap_or(Color32::WHITE);
    let changed = ui
        .horizontal(|ui| {
            ui.label(label);
            ui.color_edit_button_srgba(&mut rgba).changed()
        })
        .inner;
    if changed {
        *color = Some(ScopeColor {
            red: rgba.r(),
            green: rgba.g(),
            blue: rgba.b(),
        });
    }
    changed
}
