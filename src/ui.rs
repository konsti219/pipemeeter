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

pub fn draw_placeholder_meter(ui: &mut egui::Ui, level: f32, size: egui::Vec2) {
    const CHANNEL_WIDTH: f32 = 15.0;
    const CHANNEL_GAP: f32 = 2.0;
    const CORNER_RADIUS: f32 = 2.0;
    const INNER_MARGIN: f32 = 2.0;

    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let bg = egui::Color32::from_rgb(42, 47, 53);
    let border = egui::Color32::from_rgb(70, 77, 85);
    let fill = egui::Color32::from_rgb(92, 194, 110);

    let channels_width = CHANNEL_WIDTH * 2.0 + CHANNEL_GAP;
    let channels_left = rect.left() + ((rect.width() - channels_width) * 0.5).max(0.0);
    let left_rect = egui::Rect::from_min_size(
        egui::pos2(channels_left, rect.top()),
        egui::vec2(CHANNEL_WIDTH, rect.height()),
    );
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(channels_left + CHANNEL_WIDTH + CHANNEL_GAP, rect.top()),
        egui::vec2(CHANNEL_WIDTH, rect.height()),
    );

    for channel_rect in [left_rect, right_rect] {
        painter.rect_filled(channel_rect, CORNER_RADIUS, bg);
        painter.rect_stroke(
            channel_rect,
            CORNER_RADIUS,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Outside,
        );
    }

    let clamped = level.clamp(0.0, 1.0);
    if clamped <= 0.0 {
        return;
    }

    let fill_height = rect.height() * clamped;
    for channel_rect in [left_rect, right_rect] {
        let fill_rect = egui::Rect::from_min_max(
            egui::pos2(
                channel_rect.left() + INNER_MARGIN,
                channel_rect.bottom() - fill_height,
            ),
            egui::pos2(
                channel_rect.right() - INNER_MARGIN,
                channel_rect.bottom() - INNER_MARGIN,
            ),
        );
        painter.rect_filled(fill_rect, 1.0, fill);
    }
}
