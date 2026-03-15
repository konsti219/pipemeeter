use eframe::egui;

pub fn apply_voicemeeter_like_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(18, 22, 26);
    visuals.extreme_bg_color = egui::Color32::from_rgb(10, 14, 18);
    visuals.faint_bg_color = egui::Color32::from_rgb(28, 34, 40);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(26, 30, 35);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(35, 40, 46);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(48, 54, 62);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(58, 74, 86);
    visuals.hyperlink_color = egui::Color32::from_rgb(89, 185, 220);
    visuals.selection.bg_fill = egui::Color32::from_rgb(214, 156, 73);
    visuals.selection.stroke.color = egui::Color32::from_rgb(18, 22, 26);

    ctx.set_visuals(visuals);
}

pub fn draw_placeholder_meter(ui: &mut egui::Ui, level: f32, height: f32) {
    let size = egui::vec2(16.0, height);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let bg = egui::Color32::from_rgb(42, 47, 53);
    let border = egui::Color32::from_rgb(70, 77, 85);
    let fill = egui::Color32::from_rgb(92, 194, 110);

    painter.rect_filled(rect, 2.0, bg);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Outside,
    );

    let clamped = level.clamp(0.0, 1.0);
    if clamped <= 0.0 {
        return;
    }

    let fill_height = rect.height() * clamped;
    let fill_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 2.0, rect.bottom() - fill_height),
        egui::pos2(rect.right() - 2.0, rect.bottom() - 2.0),
    );
    painter.rect_filled(fill_rect, 1.0, fill);
}
