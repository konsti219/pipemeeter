use eframe::egui;

use crate::pipewire_backend::PwNodeCategory;
use crate::ui::draw_placeholder_meter;

use super::{Group, PipeMeeterApp, StripTarget};

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

impl PipeMeeterApp {
    pub(super) fn draw_input_subgroup(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        group: Group,
        output_labels: &[String],
        dirty: &mut bool,
    ) {
        let len = match group {
            Group::Physical => self.config.physical_inputs.len(),
            Group::Virtual => self.config.virtual_inputs.len(),
        };

        ui.vertical(|ui| {
            ui.set_width(162.0 * len.max(1) as f32 - 22.0);

            ui.horizontal(|ui| {
                ui.heading(title);
                if ui.button("+ Add").clicked() {
                    self.add_input_strip(group);
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
                    let resolved_node_title = self.resolved_node_title(target);
                    let resolved_slider_value = match group {
                        Group::Physical => self.resolved_volume_slider_value(target),
                        Group::Virtual => None,
                    };
                    let resolved_meter_level = match group {
                        Group::Physical => None,
                        Group::Virtual => self.resolved_meter_level(target),
                    };
                    let resolved_node_ids = self.resolved_node_ids(target);
                    let mut open_dialog = false;
                    let mut changed_volume = None;

                    let strip = match group {
                        Group::Physical => &mut self.config.physical_inputs[index],
                        Group::Virtual => &mut self.config.virtual_inputs[index],
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
                            draw_placeholder_meter(
                                ui,
                                resolved_meter_level
                                    .unwrap_or([strip.placeholder_meter, strip.placeholder_meter]),
                                egui::vec2(32.0, 250.0),
                            );
                            let mut slider_value = resolved_slider_value.unwrap_or(strip.volume);
                            let slider = egui::Slider::new(&mut slider_value, 0.0..=1.0)
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
                            draw_slider_knob_percentage(ui, slider_rect, slider_value);
                            if slider_changed {
                                strip.volume = slider_value;
                                changed_volume = Some(slider_value);
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
                                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
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
                        if category == PwNodeCategory::PlaybackStream {
                            self.apply_virtual_input_slider_volume(index, volume);
                        } else {
                            for node_id in resolved_node_ids {
                                let linear = super::volume::human_slider_to_pipewire_linear(volume);
                                if let Err(err) = self.backend.set_node_volume(node_id, linear) {
                                    self.status = format!(
                                        "failed to set input volume for node #{}: {err}",
                                        node_id
                                    );
                                }
                            }
                        }
                    }

                    if open_dialog {
                        self.open_edit_dialog(target);
                    }
                }
            });
        });
    }

    pub(super) fn draw_output_subgroup(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        group: Group,
        dirty: &mut bool,
    ) {
        let len = match group {
            Group::Physical => self.config.physical_outputs.len(),
            Group::Virtual => self.config.virtual_outputs.len(),
        };

        ui.vertical(|ui| {
            ui.set_width(112.0 * len.max(1) as f32 - 22.0);

            ui.horizontal(|ui| {
                ui.heading(title);
                if ui.button("+ Add").clicked() {
                    self.add_output_strip(group);
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
                    let resolved_node_title = self.resolved_node_title(target);
                    let resolved_slider_value = self.resolved_volume_slider_value(target);
                    let resolved_meter_level = match group {
                        Group::Physical => None,
                        Group::Virtual => self.resolved_meter_level(target),
                    };
                    let resolved_node_ids = self.resolved_node_ids(target);
                    let mut open_dialog = false;
                    let mut changed_volume = None;

                    let strip = match group {
                        Group::Physical => &mut self.config.physical_outputs[index],
                        Group::Virtual => &mut self.config.virtual_outputs[index],
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
                            draw_placeholder_meter(
                                ui,
                                resolved_meter_level
                                    .unwrap_or([strip.placeholder_meter, strip.placeholder_meter]),
                                egui::vec2(32.0, 250.0),
                            );
                            let mut slider_value = resolved_slider_value.unwrap_or(strip.volume);
                            let slider = egui::Slider::new(&mut slider_value, 0.0..=1.0)
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
                            draw_slider_knob_percentage(ui, slider_rect, slider_value);
                            if slider_changed {
                                strip.volume = slider_value;
                                changed_volume = Some(slider_value);
                                *dirty = true;
                            }
                        });
                    });

                    if index != len - 1 {
                        ui.separator();
                    }

                    if let Some(volume) = changed_volume {
                        for node_id in resolved_node_ids {
                            let linear = super::volume::human_slider_to_pipewire_linear(volume);
                            if let Err(err) = self.backend.set_node_volume(node_id, linear) {
                                self.status = format!(
                                    "failed to set output volume for node #{}: {err}",
                                    node_id
                                );
                            }
                        }
                    }

                    if open_dialog {
                        self.open_edit_dialog(target);
                    }
                }
            });
        });
    }
}
