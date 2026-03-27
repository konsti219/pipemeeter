use eframe::egui;

use crate::{
    config::{AppConfig, StripConfig},
    pipewire_backend::{PwNodeCategory, PwState},
    volume::slider_to_pipewire_linear,
};

use super::{Group, PipeMeeterApp, StripTarget};

impl PipeMeeterApp {
    pub(super) fn draw_input_subgroup(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        group: Group,
        output_labels: &[String],
        config: &mut AppConfig,
        objects: &PwState,
        dirty: &mut bool,
    ) {
        let len = match group {
            Group::Physical => config.physical_inputs.len(),
            Group::Virtual => config.virtual_inputs.len(),
        };

        ui.vertical(|ui| {
            ui.set_width(162.0 * len.max(1) as f32 - 22.0);
            let mut add_requested = false;
            let mut open_dialog_target = None;

            ui.horizontal(|ui| {
                ui.heading(title);
                if ui.button("+ Add").clicked() {
                    add_requested = true;
                }
            });

            ui.separator();

            ui.horizontal(|ui| {
                let category = match group {
                    Group::Physical => PwNodeCategory::InputDevice,
                    Group::Virtual => PwNodeCategory::PlaybackStream,
                };

                for index in 0..len {
                    let target = StripTarget::new(index, category);
                    let resolved_node_title =
                        Self::resolved_node_title_from_state(config, objects, target);
                    let resolved_meter_level =
                        self.resolved_meter_level_from_config(config, target);
                    let mut open_dialog = false;
                    let mut changed_volume = None;

                    let Some(strip) = (match group {
                        Group::Physical => config.physical_inputs.get_mut(index),
                        Group::Virtual => config.virtual_inputs.get_mut(index),
                    }) else {
                        continue;
                    };

                    ui.vertical(|ui| {
                        ui.set_width(140.0);

                        if let Some((line1, line2)) = resolved_node_title {
                            if draw_strip_header(ui, &strip.name, &line1, line2.as_deref(), false) {
                                open_dialog = true;
                            }
                        } else {
                            let req = strip.requirements.first().map(|req| req.pattern.as_str());

                            if draw_strip_header(ui, &strip.name, "No match", req, true) {
                                open_dialog = true;
                            }
                        }

                        ui.separator();
                        ui.add_space(3.0);

                        ui.horizontal(|ui| {
                            draw_volume_meter(ui, resolved_meter_level, egui::vec2(32.0, 250.0));
                            let slider = egui::Slider::new(&mut strip.volume, 0.0..=1.0)
                                .step_by(0.05)
                                .vertical()
                                .show_value(false);
                            let (slider_changed, slider_rect) = ui
                                .scope(|ui| {
                                    let spacing = ui.spacing_mut();
                                    spacing.slider_width = 250.0;
                                    spacing.interact_size.y = 40.0;
                                    let response = ui.add(slider);
                                    (response.changed(), response.rect)
                                })
                                .inner;
                            draw_slider_knob_percentage(ui, slider_rect, strip.volume);
                            if slider_changed {
                                changed_volume = Some(strip.volume);
                                *dirty = true;
                            }

                            if output_labels.is_empty() {
                                ui.label("No outputs");
                            }
                            ui.vertical(|ui| {
                                for (route_index, output_label) in output_labels.iter().enumerate()
                                {
                                    if let Some(route) =
                                        strip.routes_to_outputs.get_mut(route_index)
                                    {
                                        ui.style_mut().wrap_mode =
                                            Some(egui::TextWrapMode::Truncate);
                                        if ui.checkbox(route, output_label).changed() {
                                            *dirty = true;
                                        }
                                    }
                                }
                            });
                        })
                    });

                    if index != len - 1 {
                        ui.separator();
                    }

                    if let Some(volume) = changed_volume {
                        let linear = slider_to_pipewire_linear(volume);

                        if group == Group::Virtual {
                            if let Some(node_id) = self.virtual_input_node_id(index) {
                                self.backend.set_node_volume(node_id, linear).unwrap();
                            };
                        } else {
                            for node_id in &strip.resolved_nodes {
                                self.backend.set_node_volume(*node_id, linear).unwrap();
                            }
                        }
                    }

                    if open_dialog {
                        open_dialog_target = Some(target);
                    }
                }
            });

            if add_requested {
                let output_count = config.output_count();
                let strip = match group {
                    Group::Physical => {
                        let name = Self::default_input_name(group, config.physical_inputs.len());
                        StripConfig::with_routes(name, output_count)
                    }
                    Group::Virtual => {
                        let name = Self::default_input_name(group, config.virtual_inputs.len());
                        StripConfig::with_routes(name, output_count)
                    }
                };

                match group {
                    Group::Physical => config.physical_inputs.push(strip),
                    Group::Virtual => config.virtual_inputs.push(strip),
                }
                *dirty = true;
            }

            if let Some(target) = open_dialog_target {
                self.open_edit_dialog_from_config(target, config);
            }
        });
    }

    pub(super) fn draw_output_subgroup(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        group: Group,
        config: &mut AppConfig,
        objects: &PwState,
        dirty: &mut bool,
    ) {
        let len = match group {
            Group::Physical => config.physical_outputs.len(),
            Group::Virtual => config.virtual_outputs.len(),
        };

        ui.vertical(|ui| {
            ui.set_width(112.0 * len.max(1) as f32 - 22.0);
            let mut add_requested = false;
            let mut open_dialog_target = None;

            ui.horizontal(|ui| {
                ui.heading(title);
                if ui.button("+ Add").clicked() {
                    add_requested = true;
                }
            });

            ui.separator();

            ui.horizontal(|ui| {
                let category = match group {
                    Group::Physical => PwNodeCategory::OutputDevice,
                    Group::Virtual => PwNodeCategory::RecordingStream,
                };

                for index in 0..len {
                    let target = StripTarget::new(index, category);
                    let resolved_node_title =
                        Self::resolved_node_title_from_state(config, objects, target);
                    let resolved_meter_level =
                        self.resolved_meter_level_from_config(config, target);
                    let mut open_dialog = false;
                    let mut changed_volume = None;

                    let Some(strip) = (match group {
                        Group::Physical => config.physical_outputs.get_mut(index),
                        Group::Virtual => config.virtual_outputs.get_mut(index),
                    }) else {
                        continue;
                    };

                    ui.vertical(|ui| {
                        ui.set_width(90.0);

                        if let Some((line1, line2)) = resolved_node_title {
                            if draw_strip_header(ui, &strip.name, &line1, line2.as_deref(), false) {
                                open_dialog = true;
                            }
                        } else {
                            let req = strip.requirements.first().map(|req| req.pattern.as_str());

                            if draw_strip_header(ui, &strip.name, "No match", req, true) {
                                open_dialog = true;
                            }
                        }

                        ui.separator();
                        ui.add_space(3.0);

                        ui.horizontal(|ui| {
                            draw_volume_meter(ui, resolved_meter_level, egui::vec2(32.0, 250.0));
                            let slider = egui::Slider::new(&mut strip.volume, 0.0..=1.0)
                                .step_by(0.05)
                                .vertical()
                                .show_value(false);
                            let (slider_changed, slider_rect) = ui
                                .scope(|ui| {
                                    let spacing = ui.spacing_mut();
                                    spacing.slider_width = 250.0;
                                    spacing.interact_size.y = 40.0;
                                    let response = ui.add(slider);
                                    (response.changed(), response.rect)
                                })
                                .inner;
                            draw_slider_knob_percentage(ui, slider_rect, strip.volume);
                            if slider_changed {
                                changed_volume = Some(strip.volume);
                                *dirty = true;
                            }
                        });
                    });

                    if index != len - 1 {
                        ui.separator();
                    }

                    if let Some(volume) = changed_volume {
                        let linear = slider_to_pipewire_linear(volume);
                        if group == Group::Virtual {
                            if let Some(node_id) = self.virtual_output_node_id(index) {
                                self.backend.set_node_volume(node_id, linear).unwrap();
                            }
                        } else {
                            for node_id in &strip.resolved_nodes {
                                self.backend.set_node_volume(*node_id, linear).unwrap()
                            }
                        }
                    }

                    if open_dialog {
                        open_dialog_target = Some(target);
                    }
                }
            });

            if add_requested {
                match group {
                    Group::Physical => {
                        let name = Self::default_output_name(group, config.physical_outputs.len());
                        config.physical_outputs.push(StripConfig::new(name));
                    }
                    Group::Virtual => {
                        let name = Self::default_output_name(group, config.virtual_outputs.len());
                        config.virtual_outputs.push(StripConfig::new(name));
                    }
                }
                config.normalize();
                *dirty = true;
            }

            if let Some(target) = open_dialog_target {
                self.open_edit_dialog_from_config(target, config);
            }
        });
    }
}

fn draw_strip_header(
    ui: &mut egui::Ui,
    strip_name: &str,
    first_line: &str,
    second_line: Option<&str>,
    unresolved: bool,
) -> bool {
    let mut open_dialog = false;

    ui.horizontal(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.label(egui::RichText::new(strip_name).strong());

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let gear = egui::Button::new(egui::RichText::new("⚙").size(14.0))
                .min_size(egui::vec2(22.0, 22.0));
            if ui.add(gear).clicked() {
                open_dialog = true;
            }
        });
    });

    if let Some(second_line) = second_line {
        let color = if unresolved {
            egui::Color32::RED
        } else {
            ui.visuals().text_color()
        };

        ui.scope(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.colored_label(color, first_line);
            ui.colored_label(color, second_line);
        });
    } else {
        let node_title_height = ui.text_style_height(&egui::TextStyle::Body) * 2.0;
        let width = ui.available_width();
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(width, node_title_height), egui::Sense::hover());
        let mut layout_job = egui::text::LayoutJob::default();
        layout_job.append(
            first_line,
            0.0,
            egui::TextFormat {
                color: if unresolved {
                    egui::Color32::RED
                } else {
                    ui.visuals().text_color()
                },
                ..egui::TextFormat::default()
            },
        );
        layout_job.wrap.max_width = width;
        layout_job.wrap.max_rows = 2;
        let galley = ui.painter().layout_job(layout_job);
        ui.painter().with_clip_rect(rect).galley(
            rect.min,
            galley,
            if unresolved {
                egui::Color32::RED
            } else {
                ui.visuals().text_color()
            },
        );
    }

    open_dialog
}

fn draw_slider_knob_percentage(ui: &egui::Ui, slider_rect: egui::Rect, slider_value: f32) {
    let t = slider_value.clamp(0.0, 1.0);
    let base_handle_radius = slider_rect.width() / 2.5;
    let handle_extent = match ui.style().visuals.handle_shape {
        egui::style::HandleShape::Circle => base_handle_radius,
        egui::style::HandleShape::Rect { aspect_ratio } => base_handle_radius * aspect_ratio,
    };
    let y_top = slider_rect.top() + handle_extent;
    let y_bottom = slider_rect.bottom() - handle_extent;
    let knob_y = egui::lerp(y_bottom..=y_top, t);
    let percent = (t * 100.0).round() as i32;

    ui.painter().text(
        egui::pos2(slider_rect.center().x, knob_y),
        egui::Align2::CENTER_CENTER,
        format!("{percent}"),
        egui::FontId::proportional(11.0),
        ui.visuals().widgets.active.fg_stroke.color,
    );
}

pub fn draw_volume_meter(ui: &mut egui::Ui, levels: [f32; 2], size: egui::Vec2) {
    const CHANNEL_WIDTH: f32 = 15.0;
    const CHANNEL_GAP: f32 = 2.0;
    const CORNER_RADIUS: f32 = 2.0;
    const INNER_MARGIN: f32 = 2.0;

    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let bg = egui::Color32::from_rgb(42, 47, 53);
    let border = egui::Color32::from_rgb(70, 77, 85);
    let fill_green = egui::Color32::from_rgb(92, 194, 110);
    let fill_yellow = egui::Color32::from_rgb(223, 208, 97);

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

    for (channel_rect, level) in [(left_rect, levels[0]), (right_rect, levels[1])] {
        let clamped = level.clamp(0.0, 1.0);
        if clamped <= 0.0 {
            continue;
        }

        let fill_height = rect.height() * clamped;
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
        let color = if level < 1.0 { fill_green } else { fill_yellow };
        painter.rect_filled(fill_rect, 1.0, color);
    }
}
