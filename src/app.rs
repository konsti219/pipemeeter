mod dialog_ui;
mod node_resolution;
mod strip_ui;
mod types;

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui;
use log::error;

use crate::config::{
    AppConfig, InputStripConfig, OutputStripConfig, config_path, load_config, save_config,
};
use crate::pipewire_backend::{PipewireBackend, PwStateExt};
use crate::ui::apply_voicemeeter_like_theme;
use types::{EditDialogState, Group, ResolvedNodeInfo, StripTarget};

pub struct PipeMeeterApp {
    config_path: PathBuf,
    config: AppConfig,

    backend: PipewireBackend,
    resolved_nodes: HashMap<StripTarget, ResolvedNodeInfo>,

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
            resolved_nodes: HashMap::new(),
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
        let _width = (input_strips * 150.0 + output_strips * 100.0)
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
                self.config.physical_outputs.push(OutputStripConfig::new(name));
            }
            Group::Virtual => {
                let name = Self::default_output_name(group, self.config.virtual_outputs.len());
                self.config.virtual_outputs.push(OutputStripConfig::new(name));
            }
        }

        self.config.normalize();
        self.persist_config();
    }

    fn input_strip_names(&self, group: Group, index: usize) -> Option<(String, String)> {
        match group {
            Group::Physical => self
                .config
                .physical_inputs
                .get(index)
                .map(|s| (s.name.clone(), s.represented_node_name.clone())),
            Group::Virtual => self
                .config
                .virtual_inputs
                .get(index)
                .map(|s| (s.name.clone(), s.represented_node_name.clone())),
        }
    }

    fn output_strip_names(&self, group: Group, index: usize) -> Option<(String, String)> {
        match group {
            Group::Physical => self
                .config
                .physical_outputs
                .get(index)
                .map(|s| (s.name.clone(), s.represented_node_name.clone())),
            Group::Virtual => self
                .config
                .virtual_outputs
                .get(index)
                .map(|s| (s.name.clone(), s.represented_node_name.clone())),
        }
    }

    fn open_edit_dialog(&mut self, target: StripTarget) {
        let draft_names = match target {
            StripTarget::Input { group, index } => self.input_strip_names(group, index),
            StripTarget::Output { group, index } => self.output_strip_names(group, index),
        };

        if let Some((strip_name, represented_node_name)) = draft_names {
            self.edit_dialog = Some(EditDialogState {
                target,
                draft_strip_name: strip_name,
                draft_represented_node_name: represented_node_name,
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

    fn apply_dialog_update(
        &mut self,
        target: StripTarget,
        strip_name: String,
        represented_node_name: String,
    ) {
        let trimmed_strip_name = strip_name.trim();
        if trimmed_strip_name.is_empty() {
            self.status = "name cannot be empty".to_owned();
            return;
        }

        let normalized_node_name = represented_node_name.trim().to_owned();

        match target {
            StripTarget::Input { group, index } => match group {
                Group::Physical => {
                    if let Some(strip) = self.config.physical_inputs.get_mut(index) {
                        strip.name = trimmed_strip_name.to_owned();
                        strip.represented_node_name = normalized_node_name.clone();
                    }
                }
                Group::Virtual => {
                    if let Some(strip) = self.config.virtual_inputs.get_mut(index) {
                        strip.name = trimmed_strip_name.to_owned();
                        strip.represented_node_name = normalized_node_name.clone();
                    }
                }
            },
            StripTarget::Output { group, index } => match group {
                Group::Physical => {
                    if let Some(strip) = self.config.physical_outputs.get_mut(index) {
                        strip.name = trimmed_strip_name.to_owned();
                        strip.represented_node_name = normalized_node_name.clone();
                    }
                }
                Group::Virtual => {
                    if let Some(strip) = self.config.virtual_outputs.get_mut(index) {
                        strip.name = trimmed_strip_name.to_owned();
                        strip.represented_node_name = normalized_node_name;
                    }
                }
            },
        }

        self.persist_config();
    }
}

impl eframe::App for PipeMeeterApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();

        apply_voicemeeter_like_theme(ctx);
        self.apply_viewport_size(ctx);
        self.refresh_resolved_nodes();

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
