use std::path::PathBuf;

use eframe::egui;
use log::error;

use crate::config::{
    AppConfig, InputStripConfig, OutputStripConfig, config_path, load_config, save_config,
};
use crate::pipewire_backend::{PipewireBackend, PwNodeCategory, PwStateExt};
use crate::ui::{apply_voicemeeter_like_theme, draw_placeholder_meter};

#[derive(Debug, Clone, Copy)]
enum Group {
    Physical,
    Virtual,
}

#[derive(Debug, Clone, Copy)]
enum StripTarget {
    Input { group: Group, index: usize },
    Output { group: Group, index: usize },
}

#[derive(Debug, Clone)]
struct EditDialogState {
    target: StripTarget,
    draft_name: String,
}

pub struct PipeMeeterApp {
    config_path: PathBuf,
    config: AppConfig,

    backend: PipewireBackend,

    status: String,
    edit_dialog: Option<EditDialogState>,
    last_viewport_size: Option<egui::Vec2>,
}

impl PipeMeeterApp {
    pub fn new() -> Self {
        let config_path = match config_path() {
            Ok(path) => path,
            Err(err) => {
                error!("failed to resolve config path, falling back to local file: {err}");
                PathBuf::from("./pipemeeter-config.json")
            }
        };

        let mut config = match load_config(&config_path) {
            Ok(config) => config,
            Err(err) => {
                error!(
                    "failed to load config at {}: {err}; using defaults",
                    config_path.display()
                );
                AppConfig::default()
            }
        };
        config.normalize();

        let backend = PipewireBackend::new().unwrap();

        Self {
            config_path,
            config,
            backend,
            status: "UI-only setup mode (backend disabled)".to_owned(),
            edit_dialog: None,
            last_viewport_size: None,
        }
    }

    pub fn desired_viewport_size(&self) -> egui::Vec2 {
        const GAP: f32 = 22.0;

        let input_strips = self.config.physical_inputs.len().max(1) as f32
            + self.config.virtual_inputs.len().max(1) as f32;
        let output_strips = (self.config.physical_outputs.len() as f32).max(1.35)
            + (self.config.virtual_outputs.len() as f32).max(1.35);
        let width = (input_strips * 150.0 + output_strips * 100.0)
            + GAP * (input_strips + output_strips - 1.0);

        // egui::vec2(width + 16.0, 450.0)
        egui::vec2(2500.0, 1200.0)
    }

    fn apply_viewport_size(&mut self, ctx: &egui::Context) {
        let target = self.desired_viewport_size();
        let should_resize = self
            .last_viewport_size
            .map(|last| (last.x - target.x).abs() > 1.0 || (last.y - target.y).abs() > 1.0)
            .unwrap_or(true);

        if should_resize {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target));
            self.last_viewport_size = Some(target);
        }
    }

    fn persist_config(&mut self) {
        self.config.normalize();
        match save_config(&self.config_path, &self.config) {
            Ok(()) => {
                self.status = format!("saved setup to {}", self.config_path.display());
            }
            Err(err) => {
                self.status = format!("failed to save config: {err}");
                error!("{}", self.status);
            }
        }
    }

    fn default_input_name(group: Group, count: usize) -> String {
        match group {
            Group::Physical => format!("Phys In {}", count + 1),
            Group::Virtual => format!("Virt In {}", count + 1),
        }
    }

    fn default_output_name(group: Group, count: usize) -> String {
        match group {
            Group::Physical => format!("Phys Out {}", count + 1),
            Group::Virtual => format!("Virt Out {}", count + 1),
        }
    }

    fn add_input_strip(&mut self, group: Group) {
        let output_count = self.config.output_count();
        match group {
            Group::Physical => {
                let name = Self::default_input_name(group, self.config.physical_inputs.len());
                self.config
                    .physical_inputs
                    .push(InputStripConfig::new(name, output_count));
            }
            Group::Virtual => {
                let name = Self::default_input_name(group, self.config.virtual_inputs.len());
                self.config
                    .virtual_inputs
                    .push(InputStripConfig::new(name, output_count));
            }
        }
        self.persist_config();
    }

    fn add_output_strip(&mut self, group: Group) {
        match group {
            Group::Physical => {
                let name = Self::default_output_name(group, self.config.physical_outputs.len());
                self.config
                    .physical_outputs
                    .push(OutputStripConfig::new(name));
            }
            Group::Virtual => {
                let name = Self::default_output_name(group, self.config.virtual_outputs.len());
                self.config
                    .virtual_outputs
                    .push(OutputStripConfig::new(name));
            }
        }

        self.config.normalize();
        self.persist_config();
    }

    fn input_name(&self, group: Group, index: usize) -> Option<String> {
        match group {
            Group::Physical => self
                .config
                .physical_inputs
                .get(index)
                .map(|s| s.name.clone()),
            Group::Virtual => self
                .config
                .virtual_inputs
                .get(index)
                .map(|s| s.name.clone()),
        }
    }

    fn output_name(&self, group: Group, index: usize) -> Option<String> {
        match group {
            Group::Physical => self
                .config
                .physical_outputs
                .get(index)
                .map(|s| s.name.clone()),
            Group::Virtual => self
                .config
                .virtual_outputs
                .get(index)
                .map(|s| s.name.clone()),
        }
    }

    fn open_edit_dialog(&mut self, target: StripTarget) {
        let draft_name = match target {
            StripTarget::Input { group, index } => self.input_name(group, index),
            StripTarget::Output { group, index } => self.output_name(group, index),
        };

        if let Some(name) = draft_name {
            self.edit_dialog = Some(EditDialogState {
                target,
                draft_name: name,
            });
        }
    }

    fn global_output_index(&self, group: Group, index: usize) -> usize {
        match group {
            Group::Physical => index,
            Group::Virtual => self.config.physical_outputs.len() + index,
        }
    }

    fn delete_target(&mut self, target: StripTarget) {
        match target {
            StripTarget::Input { group, index } => match group {
                Group::Physical => {
                    if index < self.config.physical_inputs.len() {
                        self.config.physical_inputs.remove(index);
                        self.persist_config();
                    }
                }
                Group::Virtual => {
                    if self.config.virtual_inputs.len() == 1 {
                        self.status =
                            "cannot delete the last virtual input (at least one is required)"
                                .to_owned();
                        return;
                    }
                    if index < self.config.virtual_inputs.len() {
                        self.config.virtual_inputs.remove(index);
                        self.persist_config();
                    }
                }
            },
            StripTarget::Output { group, index } => {
                let output_idx = self.global_output_index(group, index);

                match group {
                    Group::Physical => {
                        if index < self.config.physical_outputs.len() {
                            self.config.physical_outputs.remove(index);
                        } else {
                            return;
                        }
                    }
                    Group::Virtual => {
                        if index < self.config.virtual_outputs.len() {
                            self.config.virtual_outputs.remove(index);
                        } else {
                            return;
                        }
                    }
                }

                for input in self
                    .config
                    .physical_inputs
                    .iter_mut()
                    .chain(self.config.virtual_inputs.iter_mut())
                {
                    if output_idx < input.routes_to_outputs.len() {
                        input.routes_to_outputs.remove(output_idx);
                    }
                }

                self.persist_config();
            }
        }
    }

    fn apply_dialog_rename(&mut self, target: StripTarget, name: String) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            self.status = "name cannot be empty".to_owned();
            return;
        }

        match target {
            StripTarget::Input { group, index } => match group {
                Group::Physical => {
                    if let Some(strip) = self.config.physical_inputs.get_mut(index) {
                        strip.name = trimmed.to_owned();
                    }
                }
                Group::Virtual => {
                    if let Some(strip) = self.config.virtual_inputs.get_mut(index) {
                        strip.name = trimmed.to_owned();
                    }
                }
            },
            StripTarget::Output { group, index } => match group {
                Group::Physical => {
                    if let Some(strip) = self.config.physical_outputs.get_mut(index) {
                        strip.name = trimmed.to_owned();
                    }
                }
                Group::Virtual => {
                    if let Some(strip) = self.config.virtual_outputs.get_mut(index) {
                        strip.name = trimmed.to_owned();
                    }
                }
            },
        }

        self.persist_config();
    }

    fn draw_input_subgroup(
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
            ui.set_width(172.0 * len.max(1) as f32 - 22.0);

            ui.horizontal(|ui| {
                ui.heading(title);
                if ui.button("+ Add").clicked() {
                    self.add_input_strip(group);
                }
            });

            ui.separator();

            ui.horizontal(|ui| {
                for index in 0..len {
                    let mut open_dialog = false;

                    let strip = match group {
                        Group::Physical => &mut self.config.physical_inputs[index],
                        Group::Virtual => &mut self.config.virtual_inputs[index],
                    };

                    ui.vertical(|ui| {
                        ui.set_width(150.0);

                        ui.horizontal(|ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            ui.label(egui::RichText::new(strip.name.clone()).strong());

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let gear =
                                        egui::Button::new(egui::RichText::new("⚙").size(14.0))
                                            .min_size(egui::vec2(22.0, 22.0));
                                    if ui.add(gear).clicked() {
                                        open_dialog = true;
                                    }
                                },
                            );
                        });
                        ui.separator();
                        ui.add_space(3.0);

                        ui.horizontal(|ui| {
                            draw_placeholder_meter(ui, strip.placeholder_meter, 160.0);
                            let slider = egui::Slider::new(&mut strip.volume, 0.0..=1.0)
                                .vertical()
                                .show_value(false);
                            if ui.add(slider).changed() {
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

                    if open_dialog {
                        self.open_edit_dialog(StripTarget::Input { group, index });
                    }
                }
            });
        });
    }

    fn draw_output_subgroup(
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
            ui.set_width(122.0 * len.max(1) as f32 - 22.0);

            ui.horizontal(|ui| {
                ui.heading(title);
                if ui.button("+ Add").clicked() {
                    self.add_output_strip(group);
                }
            });

            ui.separator();

            ui.horizontal(|ui| {
                for index in 0..len {
                    let mut open_dialog = false;

                    let strip = match group {
                        Group::Physical => &mut self.config.physical_outputs[index],
                        Group::Virtual => &mut self.config.virtual_outputs[index],
                    };

                    ui.vertical(|ui| {
                        ui.set_width(100.0);

                        ui.horizontal(|ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            ui.label(egui::RichText::new(strip.name.clone()).strong());

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let gear =
                                        egui::Button::new(egui::RichText::new("⚙").size(14.0))
                                            .min_size(egui::vec2(22.0, 22.0));
                                    if ui.add(gear).clicked() {
                                        open_dialog = true;
                                    }
                                },
                            );
                        });
                        ui.separator();
                        ui.add_space(3.0);

                        ui.horizontal(|ui| {
                            draw_placeholder_meter(ui, strip.placeholder_meter, 160.0);
                            let slider = egui::Slider::new(&mut strip.volume, 0.0..=1.0)
                                .vertical()
                                .show_value(false);
                            if ui.add(slider).changed() {
                                *dirty = true;
                            }
                        });
                    });

                    if index != len - 1 {
                        ui.separator();
                    }

                    if open_dialog {
                        self.open_edit_dialog(StripTarget::Output { group, index });
                    }
                }
            });
        });
    }

    fn show_edit_dialog(&mut self, ctx: &egui::Context) {
        enum DialogAction {
            Save,
            Delete,
            Cancel,
        }

        let mut dialog = if let Some(dialog) = &mut self.edit_dialog {
            dialog.clone()
        } else {
            return;
        };

        let mut is_open = true;
        let mut action = None;
        let mut new_name = String::new();

        let objects = self.backend.objects.lock().unwrap();
        let filter = match dialog.target {
            StripTarget::Input { group, .. } => match group {
                Group::Physical => PwNodeCategory::InputDevice,
                Group::Virtual => PwNodeCategory::PlaybackStream,
            },
            StripTarget::Output { group, .. } => match group {
                Group::Physical => PwNodeCategory::OutputDevice,
                Group::Virtual => PwNodeCategory::RecordingStream,
            },
        };

        egui::Window::new("Configure Strip")
            .collapsible(false)
            .resizable(false)
            .open(&mut is_open)
            .show(ctx, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut dialog.draft_name);
                ui.add_space(8.0);

                ui.label("Available nodes");
                ui.add_space(4.0);

                let mut nodes = objects
                    .nodes()
                    .filter(|node| node.category == filter)
                    .collect::<Vec<_>>();
                nodes.sort_by_key(|node| node.id);

                egui::ScrollArea::vertical()
                    .max_height(140.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if nodes.is_empty() {
                            ui.label("No audio nodes found.");
                        } else {
                            for node in nodes {
                                ui.label(format!(
                                    "#{} {} ({:?})({:?}) {:?}",
                                    node.id,
                                    node.name,
                                    node.description,
                                    node.media_name,
                                    node.volume
                                ));
                            }
                        }
                    });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        new_name = dialog.draft_name.clone();
                        action = Some(DialogAction::Save);
                    }
                    if ui.button("Delete").clicked() {
                        action = Some(DialogAction::Delete);
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(DialogAction::Cancel);
                    }
                });
            });

        drop(objects);

        if !is_open && action.is_none() {
            action = Some(DialogAction::Cancel);
        }

        match action {
            Some(DialogAction::Save) => {
                self.apply_dialog_rename(dialog.target, new_name);
                self.edit_dialog = None;
            }
            Some(DialogAction::Delete) => {
                self.delete_target(dialog.target);
                self.edit_dialog = None;
            }
            Some(DialogAction::Cancel) => {
                self.edit_dialog = None;
            }
            None => {
                self.edit_dialog = Some(dialog);
            }
        }
    }
}

impl eframe::App for PipeMeeterApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        // ctx.set_debug_on_hover(true);

        apply_voicemeeter_like_theme(ctx);
        self.apply_viewport_size(ctx);

        let mut dirty = false;
        let output_labels = self.config.output_labels();

        egui::TopBottomPanel::bottom("status_footer")
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(self.status.clone());
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.set_height(300.0);
                    self.draw_input_subgroup(
                        ui,
                        "Physical In",
                        Group::Physical,
                        &output_labels,
                        &mut dirty,
                    );
                    ui.separator();
                    self.draw_input_subgroup(
                        ui,
                        "Virtual In",
                        Group::Virtual,
                        &output_labels,
                        &mut dirty,
                    );

                    ui.separator();

                    self.draw_output_subgroup(ui, "Physical Out", Group::Physical, &mut dirty);
                    ui.separator();
                    self.draw_output_subgroup(ui, "Virtual Out", Group::Virtual, &mut dirty);
                });
            });

            ui.separator();

            let mut node_lines = {
                let objects = self.backend.objects.lock().unwrap();
                objects
                    .nodes()
                    .map(|node| format!("{node:?}"))
                    .collect::<Vec<_>>()
            };
            node_lines.sort();

            ui.label(egui::RichText::new(format!("Current Nodes ({})", node_lines.len())).strong());
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .id_salt("node_dump_scroll")
                .max_height(1000.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if node_lines.is_empty() {
                        ui.label("No nodes currently available.");
                    } else {
                        for line in node_lines {
                            ui.monospace(line);
                        }
                    }
                });
        });

        if dirty {
            self.persist_config();
        }

        self.show_edit_dialog(ctx);
    }
}
