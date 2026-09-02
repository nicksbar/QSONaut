use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn draw_workspace(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        snapshot: &GuiState,
    ) {
        match self.workspace_mode {
            WorkspaceMode::Ft8 => self.draw_ft8_workspace(ui, ctx, snapshot),
            WorkspaceMode::Ft4 => self.draw_ft4_workspace(ui, snapshot),
            WorkspaceMode::Fst4 => self.draw_fst4_workspace(ui, snapshot),
            WorkspaceMode::Wspr => self.draw_wspr_workspace(ui, snapshot),
            WorkspaceMode::Jt9 => self.draw_jt9_workspace(ui, snapshot),
            WorkspaceMode::Jt65 => self.draw_jt65_workspace(ui, snapshot),
            WorkspaceMode::Q65 => self.draw_q65_workspace(ui, snapshot),
            WorkspaceMode::Cw => self.draw_cw_workspace(ui, snapshot),
            WorkspaceMode::Voice => self.draw_voice_workspace(ui, snapshot),
            WorkspaceMode::Sstv => self.draw_sstv_workspace(ui, ctx, snapshot),
            WorkspaceMode::Msk144 | WorkspaceMode::Fldigi => {
                self.draw_mfsk_mode_workspace(ui, snapshot, self.workspace_mode)
            }
        }
    }

    pub(crate) fn draw_bounded_workspace(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        snapshot: &GuiState,
    ) {
        if matches!(
            self.workspace_mode,
            WorkspaceMode::Ft8 | WorkspaceMode::Ft4 | WorkspaceMode::Voice | WorkspaceMode::Sstv
        ) {
            self.draw_workspace(ui, ctx, snapshot);
        } else {
            egui::ScrollArea::both()
                .id_salt("workspace_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| self.draw_workspace(ui, ctx, snapshot));
        }
    }

    pub(crate) fn split_decode_workspace_height(available_height: f32) -> (f32, f32) {
        const GAP: f32 = 4.0;
        const TX_MIN: f32 = 72.0;
        const TX_MAX: f32 = 180.0;
        let tx_height = (available_height * 0.22).clamp(TX_MIN, TX_MAX);
        let decode_height = (available_height - GAP - tx_height).max(0.0);
        (decode_height, tx_height)
    }
}
