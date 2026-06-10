use eframe::egui;
fn test(ui: &mut egui::Ui) {
    let rect = egui::Rect::EVERYTHING;
    let mut child_ui = ui.child_ui(rect, *ui.layout());
    child_ui.set_clip_rect(rect);
}
