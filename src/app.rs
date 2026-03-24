use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui;
use log::error;

mod dialog_ui;
mod node_resolution;
mod routing;
mod strip_ui;
mod types;
mod volume;

use crate::config::{
    AppConfig, NodeMatchRequirement, StripConfig, config_path, load_config, save_config,
};
use crate::pipewire_backend::{PipewireBackend, PwNodeCategory, PwStateExt};
use crate::ui::apply_voicemeeter_like_theme;
use types::{EditDialogState, Group, ResolvedNodeEntry, StripTarget};

pub struct PipeMeeterApp {
    config_path: PathBuf,
    config: AppConfig,

    backend: PipewireBackend,
    resolved_nodes: HashMap<StripTarget, Vec<ResolvedNodeEntry>>,

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
        backend.set_routing_config(config.clone()).unwrap();

        Self {
            config_path,
            config,
            backend,
            resolved_nodes: HashMap::new(),
            status: "Ready".to_owned(),
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
        let width = (input_strips * 140.0 + output_strips * 90.0)
            + GAP * (input_strips + output_strips - 1.0);

        egui::vec2(width + 16.0, 390.0)
        // egui::vec2(2500.0, 1200.0)
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
        self.backend
            .set_routing_config(self.config.clone())
            .unwrap();
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
                    .push(StripConfig::with_routes(name, output_count));
            }
            Group::Virtual => {
                let name = Self::default_input_name(group, self.config.virtual_inputs.len());
                self.config
                    .virtual_inputs
                    .push(StripConfig::with_routes(name, output_count));
            }
        }
        self.persist_config();
    }

    fn add_output_strip(&mut self, group: Group) {
        match group {
            Group::Physical => {
                let name = Self::default_output_name(group, self.config.physical_outputs.len());
                self.config.physical_outputs.push(StripConfig::new(name));
            }
            Group::Virtual => {
                let name = Self::default_output_name(group, self.config.virtual_outputs.len());
                self.config.virtual_outputs.push(StripConfig::new(name));
            }
        }

        self.config.normalize();
        self.persist_config();
    }

    fn strip_ref(&self, target: StripTarget) -> Option<&StripConfig> {
        match target.category {
            PwNodeCategory::InputDevice => self.config.physical_inputs.get(target.index),
            PwNodeCategory::PlaybackStream => self.config.virtual_inputs.get(target.index),
            PwNodeCategory::OutputDevice => self.config.physical_outputs.get(target.index),
            PwNodeCategory::RecordingStream => self.config.virtual_outputs.get(target.index),
            _ => None,
        }
    }

    fn strip_mut(&mut self, target: StripTarget) -> Option<&mut StripConfig> {
        match target.category {
            PwNodeCategory::InputDevice => self.config.physical_inputs.get_mut(target.index),
            PwNodeCategory::PlaybackStream => self.config.virtual_inputs.get_mut(target.index),
            PwNodeCategory::OutputDevice => self.config.physical_outputs.get_mut(target.index),
            PwNodeCategory::RecordingStream => self.config.virtual_outputs.get_mut(target.index),
            _ => None,
        }
    }

    fn open_edit_dialog(&mut self, target: StripTarget) {
        if let Some(strip) = self.strip_ref(target) {
            self.edit_dialog = Some(EditDialogState {
                target,
                draft_strip_name: strip.name.clone(),
                draft_represented_node_requirements: strip.requirements.clone(),
                draft_match_only_category: strip.match_only_category,
                selected_requirement_index: 0,
            });
        }
    }

    fn is_default_strip(&self, target: StripTarget) -> bool {
        matches!(
            target.category,
            PwNodeCategory::PlaybackStream | PwNodeCategory::RecordingStream
        ) && target.index == 0
    }

    fn delete_target(&mut self, target: StripTarget) {
        if target.index == 0
            && matches!(
                target.category,
                PwNodeCategory::PlaybackStream | PwNodeCategory::RecordingStream
            )
        {
            self.status = "cannot delete the default strip (index 0)".to_owned();
            return;
        }

        match target.category {
            PwNodeCategory::InputDevice => {
                self.config.physical_inputs.remove(target.index);
            }
            PwNodeCategory::PlaybackStream => {
                self.config.virtual_inputs.remove(target.index);
            }
            PwNodeCategory::OutputDevice | PwNodeCategory::RecordingStream => {
                let output_idx = match target.category {
                    PwNodeCategory::OutputDevice => {
                        self.config.physical_outputs.remove(target.index);
                        target.index
                    }
                    PwNodeCategory::RecordingStream => {
                        self.config.virtual_outputs.remove(target.index);
                        self.config.physical_outputs.len() + target.index
                    }
                    _ => return,
                };

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
            }
            _ => {}
        }

        self.persist_config();
    }

    fn apply_dialog_update(
        &mut self,
        target: StripTarget,
        strip_name: String,
        represented_node_requirements: Vec<NodeMatchRequirement>,
        match_only_category: bool,
    ) {
        let trimmed_strip_name = strip_name.trim();
        if trimmed_strip_name.is_empty() {
            self.status = "name cannot be empty".to_owned();
            return;
        }

        let normalized_requirements = represented_node_requirements
            .into_iter()
            .filter_map(|requirement| {
                let pattern = requirement.pattern.trim().to_owned();
                if pattern.is_empty() {
                    None
                } else {
                    Some(NodeMatchRequirement {
                        pattern,
                        match_property: requirement.match_property,
                    })
                }
            })
            .collect::<Vec<_>>();

        let fallback_only = self.is_default_strip(target);

        let applied_requirements = if fallback_only {
            Vec::new()
        } else {
            normalized_requirements
        };

        if let Some(strip) = self.strip_mut(target) {
            strip.name = trimmed_strip_name.to_owned();
            strip.requirements = applied_requirements;
            strip.match_only_category = match_only_category;
        }

        self.persist_config();
    }
}

impl eframe::App for PipeMeeterApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_voicemeeter_like_theme(ctx);
        self.apply_viewport_size(ctx);
        self.refresh_resolved_nodes();
        ctx.request_repaint();

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

            ui.add_space(5.0);
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
