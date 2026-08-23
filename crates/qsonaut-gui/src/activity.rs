use crate::band_plan::{
    WorkspaceMode, CORE_BAND_LABELS, CORE_EMCOMM_BAND_LABELS, CORE_HF_BAND_LABELS,
    CORE_VHF_BAND_LABELS, WORKSPACE_MODES,
};
use eframe::egui::{self, Color32, Pos2, Stroke};

/// The user-facing operating context. Profiles describe intent and defaults;
/// individual mode panels remain responsible for the actual radio behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperatingActivity {
    General,
    Pota,
    Sota,
    Contest,
    FieldDay,
    Dx,
    Satellite,
    Emcomm,
}

impl OperatingActivity {
    pub(super) const ALL: [Self; 8] = [
        Self::General,
        Self::Pota,
        Self::Sota,
        Self::Contest,
        Self::FieldDay,
        Self::Dx,
        Self::Satellite,
        Self::Emcomm,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Pota => "POTA",
            Self::Sota => "SOTA",
            Self::Contest => "Contest",
            Self::FieldDay => "Field Day",
            Self::Dx => "DX",
            Self::Satellite => "Satellite",
            Self::Emcomm => "EMCOMM",
        }
    }

    pub(super) fn profile(self) -> ActivityProfile {
        match self {
            Self::General => ActivityProfile {
                tx_cq: "CQ",
                bands: ActivityBandScope::AllCore,
                modes: ActivityModeScope::AllCore,
            },
            Self::Pota => ActivityProfile {
                tx_cq: "CQ POTA",
                bands: ActivityBandScope::Preferred(CORE_HF_BAND_LABELS),
                modes: ActivityModeScope::Preferred(&[
                    WorkspaceMode::Ft8,
                    WorkspaceMode::Ft4,
                    WorkspaceMode::Cw,
                ]),
            },
            Self::Sota => ActivityProfile {
                tx_cq: "CQ SOTA",
                bands: ActivityBandScope::Preferred(CORE_HF_BAND_LABELS),
                modes: ActivityModeScope::Preferred(&[
                    WorkspaceMode::Ft8,
                    WorkspaceMode::Ft4,
                    WorkspaceMode::Cw,
                ]),
            },
            Self::Contest => ActivityProfile {
                tx_cq: "CQ TEST",
                bands: ActivityBandScope::Preferred(CORE_HF_BAND_LABELS),
                modes: ActivityModeScope::Preferred(&[
                    WorkspaceMode::Ft8,
                    WorkspaceMode::Ft4,
                    WorkspaceMode::Cw,
                ]),
            },
            Self::FieldDay => ActivityProfile {
                tx_cq: "CQ FD",
                bands: ActivityBandScope::Preferred(CORE_HF_BAND_LABELS),
                modes: ActivityModeScope::Preferred(&[
                    WorkspaceMode::Ft8,
                    WorkspaceMode::Ft4,
                    WorkspaceMode::Cw,
                ]),
            },
            Self::Dx => ActivityProfile {
                tx_cq: "CQ DX",
                bands: ActivityBandScope::Preferred(CORE_HF_BAND_LABELS),
                modes: ActivityModeScope::Preferred(&[
                    WorkspaceMode::Ft8,
                    WorkspaceMode::Ft4,
                    WorkspaceMode::Cw,
                ]),
            },
            Self::Satellite => ActivityProfile {
                tx_cq: "CQ SAT",
                bands: ActivityBandScope::Preferred(CORE_VHF_BAND_LABELS),
                modes: ActivityModeScope::Preferred(&[WorkspaceMode::Cw, WorkspaceMode::Fldigi]),
            },
            Self::Emcomm => ActivityProfile {
                tx_cq: "CQ EMCOMM",
                bands: ActivityBandScope::Preferred(CORE_EMCOMM_BAND_LABELS),
                modes: ActivityModeScope::Preferred(&[WorkspaceMode::Cw, WorkspaceMode::Fldigi]),
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ActivityProfile {
    pub(super) tx_cq: &'static str,
    pub(super) bands: ActivityBandScope,
    pub(super) modes: ActivityModeScope,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ActivityBandScope {
    AllCore,
    Preferred(&'static [&'static str]),
}

impl ActivityBandScope {
    pub(super) fn labels(self) -> &'static [&'static str] {
        match self {
            Self::AllCore => CORE_BAND_LABELS,
            Self::Preferred(labels) => labels,
        }
    }

    pub(super) fn is_unrestricted(self) -> bool {
        matches!(self, Self::AllCore)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ActivityModeScope {
    AllCore,
    Preferred(&'static [WorkspaceMode]),
}

impl ActivityModeScope {
    pub(super) fn modes(self) -> &'static [WorkspaceMode] {
        match self {
            Self::AllCore => &WORKSPACE_MODES,
            Self::Preferred(modes) => modes,
        }
    }

    pub(super) fn is_unrestricted(self) -> bool {
        matches!(self, Self::AllCore)
    }
}

pub(super) fn draw_activity_icon(
    painter: &egui::Painter,
    activity: OperatingActivity,
    center: Pos2,
    color: Color32,
) {
    let stroke = Stroke::new(1.8_f32, color);
    let x = center.x;
    let y = center.y;
    match activity {
        OperatingActivity::General => {
            painter.circle_stroke(center, 7.0, stroke);
            painter.circle_filled(center, 2.5, color);
        }
        OperatingActivity::Pota => {
            painter.line_segment(
                [Pos2::new(x - 6.0, y - 8.0), Pos2::new(x - 6.0, y + 8.0)],
                stroke,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x - 5.0, y - 8.0),
                    Pos2::new(x + 7.0, y - 5.0),
                    Pos2::new(x - 5.0, y - 1.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        OperatingActivity::Sota => {
            painter.line_segment([Pos2::new(x - 9.0, y + 7.0), Pos2::new(x, y - 8.0)], stroke);
            painter.line_segment([Pos2::new(x, y - 8.0), Pos2::new(x + 9.0, y + 7.0)], stroke);
            painter.line_segment(
                [Pos2::new(x - 4.0, y + 1.0), Pos2::new(x + 2.0, y - 4.0)],
                stroke,
            );
        }
        OperatingActivity::Contest => {
            painter.line_segment(
                [Pos2::new(x - 8.0, y - 5.0), Pos2::new(x + 8.0, y - 5.0)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x - 8.0, y + 5.0), Pos2::new(x + 8.0, y + 5.0)],
                stroke,
            );
            painter.circle_filled(Pos2::new(x - 5.0, y), 2.0, color);
            painter.circle_filled(Pos2::new(x + 5.0, y), 2.0, color);
        }
        OperatingActivity::FieldDay => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(13.0, 13.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([Pos2::new(x - 4.0, y), Pos2::new(x + 4.0, y)], stroke);
            painter.line_segment([Pos2::new(x, y - 4.0), Pos2::new(x, y + 4.0)], stroke);
        }
        OperatingActivity::Dx => {
            painter.circle_stroke(center, 7.0, stroke);
            painter.line_segment(
                [Pos2::new(x - 3.0, y - 6.0), Pos2::new(x - 3.0, y + 6.0)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x + 3.0, y - 6.0), Pos2::new(x + 3.0, y + 6.0)],
                stroke,
            );
            painter.line_segment([Pos2::new(x - 7.0, y), Pos2::new(x + 7.0, y)], stroke);
        }
        OperatingActivity::Satellite => {
            painter.circle_stroke(center, 3.0, stroke);
            painter.line_segment(
                [Pos2::new(x - 8.0, y + 7.0), Pos2::new(x + 1.0, y - 2.0)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x + 4.0, y - 7.0), Pos2::new(x + 8.0, y - 3.0)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x + 6.0, y - 10.0), Pos2::new(x + 11.0, y - 5.0)],
                stroke,
            );
        }
        OperatingActivity::Emcomm => {
            painter.line_segment(
                [Pos2::new(x - 8.0, y), Pos2::new(x + 8.0, y)],
                Stroke::new(2.6_f32, color),
            );
            painter.line_segment(
                [Pos2::new(x, y - 8.0), Pos2::new(x, y + 8.0)],
                Stroke::new(2.6_f32, color),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_activity_profiles_expose_future_controls() {
        assert_eq!(OperatingActivity::Pota.profile().tx_cq, "CQ POTA");
        assert!(OperatingActivity::Pota
            .profile()
            .bands
            .labels()
            .contains(&"20m"));
        assert!(OperatingActivity::Contest
            .profile()
            .modes
            .modes()
            .contains(&WorkspaceMode::Cw));
    }

    #[test]
    fn general_activity_keeps_the_entire_core_band_scope() {
        let bands = OperatingActivity::General.profile().bands;
        assert!(bands.is_unrestricted());
        assert!(bands.labels().contains(&"60m"));
        assert!(bands.labels().contains(&"70cm"));
    }

    #[test]
    fn activity_modes_are_preferences_until_a_contest_adds_constraints() {
        assert!(OperatingActivity::General.profile().modes.is_unrestricted());
        assert!(!OperatingActivity::Pota.profile().modes.is_unrestricted());
        assert!(OperatingActivity::Pota
            .profile()
            .modes
            .modes()
            .contains(&WorkspaceMode::Cw));
    }
}
