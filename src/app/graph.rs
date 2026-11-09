//! qpwgraph-style, view-only visualisation of the PipeWire audio graph in its
//! own OS window, coloured by node category and placed by [`arrange`].

use std::collections::HashMap;

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Stroke, pos2, vec2};

mod arrange;
mod model;

use crate::config::AppConfig;
use crate::pipewire_backend::{PwNodeCategory, PwState};
use model::{GRID_STEP, GraphModel, PORT_FONT, SUB_FONT, TITLE_FONT};

const BG: Color32 = Color32::from_rgb(24, 25, 28);
const GRID: Color32 = Color32::from_rgb(35, 37, 41);
const TEXT: Color32 = Color32::from_rgb(228, 231, 235);
const TEXT_DIM: Color32 = Color32::from_rgb(146, 152, 161);

/// Bezier control offsets, in graph units.
const LINK_HANDLE_MIN: f32 = 30.0;
const LINK_HANDLE_MAX: f32 = 200.0;

const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 3.0;

pub struct GraphWindow {
    pub open: bool,
    hide_meters: bool,
    /// Node centres, straight out of [`arrange`] and recomputed only when the
    /// topology signature changes.
    positions: HashMap<u32, Pos2>,
    signature: u64,
    /// Graph-space point at the centre of the canvas.
    pan: Pos2,
    zoom: f32,
    /// Set once the user pans or zooms; the view then stays put.
    view_is_theirs: bool,
    needs_fit: bool,
}

impl Default for GraphWindow {
    fn default() -> Self {
        Self {
            open: false,
            hide_meters: true,
            positions: HashMap::new(),
            signature: 0,
            pan: Pos2::ZERO,
            zoom: 1.0,
            view_is_theirs: false,
            needs_fit: true,
        }
    }
}

impl GraphWindow {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.view_is_theirs = false;
            self.needs_fit = true;
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, state: &PwState, config: &AppConfig) {
        if !self.open {
            return;
        }

        let mut keep_open = true;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("pipemeeter_graph"),
            egui::ViewportBuilder::default()
                .with_title("Pipemeeter Graph")
                .with_app_id("pipemeeter")
                .with_inner_size([1200.0, 780.0])
                .with_active(true),
            |ui, class| {
                if matches!(class, egui::ViewportClass::EmbeddedWindow) {
                    self.contents(ui, state, config);
                    return;
                }

                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| self.contents(ui, state, config));

                if ui.input(|i| i.viewport().close_requested()) {
                    keep_open = false;
                }
            },
        );
        self.open = keep_open;
    }

    fn contents(&mut self, ui: &mut egui::Ui, state: &PwState, config: &AppConfig) {
        let model = model::build(state, config, self.hide_meters);
        let signature = model.signature();
        if signature != self.signature || self.positions.len() != model.nodes.len() {
            self.signature = signature;
            self.positions = model
                .nodes
                .iter()
                .map(|node| node.id)
                .zip(arrange::arrange(&model))
                .collect();
            self.needs_fit |= !self.view_is_theirs;
        }

        egui::Panel::top("graph_toolbar").show(ui, |ui| self.toolbar(ui, &model));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas(ui, &model));
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, model: &GraphModel) {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.hide_meters, "Hide meter nodes");
            ui.separator();
            if ui.button("Fit").clicked() {
                self.needs_fit = true;
            }
            ui.separator();
            ui.label(
                egui::RichText::new(format!(
                    "{} nodes · {} links{}",
                    model.nodes.len(),
                    model.links.len(),
                    if model.hidden_meters > 0 {
                        format!(" · {} meters hidden", model.hidden_meters)
                    } else {
                        String::new()
                    }
                ))
                .weak(),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("{:.0}%", self.zoom * 100.0)).weak());
            });
        });
        ui.add_space(3.0);
    }

    fn canvas(&mut self, ui: &mut egui::Ui, model: &GraphModel) {
        let (canvas, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, CornerRadius::ZERO, BG);

        if self.needs_fit {
            self.needs_fit = false;
            self.fit(model, canvas);
        }

        self.handle_input(ui, canvas, &response);

        let hovered = response
            .hover_pos()
            .and_then(|pos| self.node_at(model, canvas, pos));

        self.draw_grid(&painter, canvas);
        self.draw_links(&painter, model, canvas, hovered);
        self.draw_nodes(&painter, model, canvas, hovered);
        draw_legend(&painter, canvas);
    }

    fn handle_input(&mut self, ui: &mut egui::Ui, canvas: Rect, response: &egui::Response) {
        if response.hovered() {
            let (scroll, zoom_delta) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = zoom_delta * (1.0 + scroll * 0.0015);
            if (factor - 1.0).abs() > 0.0001 {
                if let Some(pointer) = response.hover_pos() {
                    let anchor = self.to_graph(pointer, canvas);
                    self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                    let after = self.to_graph(pointer, canvas);
                    self.pan += anchor - after;
                    self.view_is_theirs = true;
                }
            }
        }

        if response.dragged() {
            self.pan -= response.drag_delta() / self.zoom;
            self.view_is_theirs = true;
        }

        if response.double_clicked() {
            self.needs_fit = true;
        }
    }

    fn fit(&mut self, model: &GraphModel, canvas: Rect) {
        let Some(bounds) = model
            .nodes
            .iter()
            .map(|node| node.rect(self.pos(node.id)))
            .reduce(Rect::union)
        else {
            return;
        };
        let bounds = bounds.expand(40.0);
        self.pan = bounds.center();
        self.zoom = (canvas.width() / bounds.width())
            .min(canvas.height() / bounds.height())
            .clamp(MIN_ZOOM, 1.0);
    }

    fn pos(&self, id: u32) -> Pos2 {
        self.positions.get(&id).copied().unwrap_or(Pos2::ZERO)
    }

    /// A graph-space length in screen pixels, never thinner than a pixel.
    fn thickness(&self, length: f32) -> f32 {
        (length * self.zoom).max(1.0)
    }

    fn to_screen(&self, pos: Pos2, canvas: Rect) -> Pos2 {
        canvas.center() + (pos - self.pan) * self.zoom
    }

    fn to_graph(&self, pos: Pos2, canvas: Rect) -> Pos2 {
        self.pan + (pos - canvas.center()) / self.zoom
    }

    fn node_at(&self, model: &GraphModel, canvas: Rect, screen_pos: Pos2) -> Option<u32> {
        let pos = self.to_graph(screen_pos, canvas);
        model
            .nodes
            .iter()
            .rev()
            .find(|node| node.rect(self.pos(node.id)).contains(pos))
            .map(|node| node.id)
    }

    fn draw_grid(&self, painter: &egui::Painter, canvas: Rect) {
        let step = GRID_STEP * self.zoom;
        if step < 12.0 {
            return;
        }
        let stroke = Stroke::new(1.0, GRID);
        let origin = self.to_screen(Pos2::ZERO, canvas);

        let first_x = canvas.left() + (origin.x - canvas.left()).rem_euclid(step);
        let mut x = first_x;
        while x < canvas.right() {
            painter.line_segment([pos2(x, canvas.top()), pos2(x, canvas.bottom())], stroke);
            x += step;
        }

        let first_y = canvas.top() + (origin.y - canvas.top()).rem_euclid(step);
        let mut y = first_y;
        while y < canvas.bottom() {
            painter.line_segment([pos2(canvas.left(), y), pos2(canvas.right(), y)], stroke);
            y += step;
        }
    }

    fn draw_links(
        &self,
        painter: &egui::Painter,
        model: &GraphModel,
        canvas: Rect,
        hovered: Option<u32>,
    ) {
        for link in &model.links {
            let from = &model.nodes[link.from_node];
            let to = &model.nodes[link.to_node];
            let start = from.output_pos(self.pos(from.id), link.from_port);
            let end = to.input_pos(self.pos(to.id), link.to_port);

            let touches_hover = hovered == Some(from.id) || hovered == Some(to.id);
            let color = category_color(from.category);
            let color = if touches_hover {
                color
            } else {
                color.gamma_multiply(0.55)
            };

            // Control points in graph units, so the curve is the same shape at
            // every zoom instead of only its endpoints scaling.
            let handle = vec2(
                ((end.x - start.x).abs() * 0.5).clamp(LINK_HANDLE_MIN, LINK_HANDLE_MAX),
                0.0,
            );
            let width = if touches_hover { 2.4 } else { 1.5 };

            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [
                    self.to_screen(start, canvas),
                    self.to_screen(start + handle, canvas),
                    self.to_screen(end - handle, canvas),
                    self.to_screen(end, canvas),
                ],
                false,
                Color32::TRANSPARENT,
                Stroke::new(self.thickness(width), color),
            ));
        }
    }

    fn draw_nodes(
        &self,
        painter: &egui::Painter,
        model: &GraphModel,
        canvas: Rect,
        hovered: Option<u32>,
    ) {
        let show_text = self.zoom > 0.45;
        let show_ports = self.zoom > 0.35;

        for node in &model.nodes {
            let center = self.pos(node.id);
            let rect = node.rect(center);
            let screen = Rect::from_min_max(
                self.to_screen(rect.min, canvas),
                self.to_screen(rect.max, canvas),
            );
            if !canvas.intersects(screen) {
                continue;
            }

            let color = category_color(node.category);
            let is_hovered = hovered == Some(node.id);
            let radius = CornerRadius::same(self.thickness(5.0) as u8);

            painter.rect(
                screen,
                radius,
                mix(color, BG, 0.86),
                Stroke::new(
                    self.thickness(if is_hovered { 2.5 } else { 1.5 }),
                    if is_hovered { lighten(color) } else { color },
                ),
                egui::StrokeKind::Inside,
            );

            let header = Rect::from_min_max(
                screen.min,
                pos2(screen.right(), screen.top() + model::HEADER_H * self.zoom),
            );
            painter.rect_filled(
                header,
                CornerRadius {
                    nw: radius.nw,
                    ne: radius.ne,
                    sw: 0,
                    se: 0,
                },
                mix(color, BG, 0.5),
            );

            let pad = 7.0 * self.zoom;
            if show_text {
                let id = truncating(
                    painter,
                    pos2(header.right() - pad, header.top() + 8.0 * self.zoom),
                    Align2::RIGHT_CENTER,
                    format!("#{}", node.id),
                    SUB_FONT * self.zoom,
                    TEXT_DIM,
                    f32::INFINITY,
                );
                truncating(
                    painter,
                    pos2(header.left() + pad, header.center().y - 6.0 * self.zoom),
                    Align2::LEFT_CENTER,
                    node.title.clone(),
                    TITLE_FONT * self.zoom,
                    TEXT,
                    id.left() - pad - (header.left() + pad),
                );
                if let Some(subtitle) = &node.subtitle {
                    truncating(
                        painter,
                        pos2(header.left() + pad, header.bottom() - 8.0 * self.zoom),
                        Align2::LEFT_CENTER,
                        subtitle.clone(),
                        SUB_FONT * self.zoom,
                        TEXT_DIM,
                        screen.width() - 2.0 * pad,
                    );
                }
            }

            if !show_ports {
                continue;
            }

            let port_radius = self.thickness(3.0);
            let label_width = screen.width() / 2.0 - 2.0 * pad;

            for (side, ports) in [(1.0, &node.inputs), (-1.0, &node.outputs)] {
                for (index, port) in ports.iter().enumerate() {
                    let at = if side > 0.0 {
                        self.to_screen(node.input_pos(center, index), canvas)
                    } else {
                        self.to_screen(node.output_pos(center, index), canvas)
                    };
                    painter.circle_filled(at, port_radius, lighten(color));
                    if show_text {
                        truncating(
                            painter,
                            pos2(at.x + side * pad, at.y),
                            if side > 0.0 {
                                Align2::LEFT_CENTER
                            } else {
                                Align2::RIGHT_CENTER
                            },
                            port.clone(),
                            PORT_FONT * self.zoom,
                            TEXT_DIM,
                            label_width,
                        );
                    }
                }
            }
        }
    }
}

/// One line, ellipsised if it does not fit. Returns where it landed.
fn truncating(
    painter: &egui::Painter,
    pos: Pos2,
    anchor: Align2,
    text: String,
    font_size: f32,
    color: Color32,
    max_width: f32,
) -> Rect {
    let mut job = egui::text::LayoutJob::simple_singleline(
        text,
        egui::FontId::proportional(font_size),
        color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));

    let galley = painter.layout_job(job);
    let rect = anchor.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

fn draw_legend(painter: &egui::Painter, canvas: Rect) {
    const ENTRIES: [PwNodeCategory; 7] = [
        PwNodeCategory::InputDevice,
        PwNodeCategory::PlaybackStream,
        PwNodeCategory::PipemeeterNode,
        PwNodeCategory::PipemeeterMeter,
        PwNodeCategory::RecordingStream,
        PwNodeCategory::OutputDevice,
        PwNodeCategory::Other,
    ];

    let row_h = 16.0;
    let size = vec2(150.0, ENTRIES.len() as f32 * row_h + 12.0);
    let rect = Rect::from_min_size(canvas.left_bottom() + vec2(10.0, -size.y - 10.0), size);
    painter.rect(
        rect,
        CornerRadius::same(4),
        BG.gamma_multiply(0.92),
        Stroke::new(1.0, GRID),
        egui::StrokeKind::Inside,
    );

    for (index, category) in ENTRIES.iter().enumerate() {
        let y = rect.top() + 6.0 + row_h * (index as f32 + 0.5);
        painter.circle_filled(pos2(rect.left() + 14.0, y), 4.0, category_color(*category));
        painter.text(
            pos2(rect.left() + 26.0, y),
            Align2::LEFT_CENTER,
            category_label(*category),
            egui::FontId::proportional(PORT_FONT + 0.5),
            TEXT_DIM,
        );
    }
}

pub fn category_color(category: PwNodeCategory) -> Color32 {
    match category {
        PwNodeCategory::InputDevice => Color32::from_rgb(76, 175, 80),
        PwNodeCategory::OutputDevice => Color32::from_rgb(66, 133, 244),
        PwNodeCategory::PlaybackStream => Color32::from_rgb(245, 166, 35),
        PwNodeCategory::RecordingStream => Color32::from_rgb(171, 92, 216),
        PwNodeCategory::PipemeeterNode => Color32::from_rgb(0, 188, 212),
        PwNodeCategory::PipemeeterMeter => Color32::from_rgb(200, 110, 140),
        PwNodeCategory::Other => Color32::from_rgb(130, 130, 134),
    }
}

fn category_label(category: PwNodeCategory) -> &'static str {
    match category {
        PwNodeCategory::InputDevice => "Input device",
        PwNodeCategory::OutputDevice => "Output device",
        PwNodeCategory::PlaybackStream => "Playback stream",
        PwNodeCategory::RecordingStream => "Recording stream",
        PwNodeCategory::PipemeeterNode => "Pipemeeter node",
        PwNodeCategory::PipemeeterMeter => "Pipemeeter meter",
        PwNodeCategory::Other => "Other",
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

fn lighten(color: Color32) -> Color32 {
    mix(color, Color32::WHITE, 0.35)
}
