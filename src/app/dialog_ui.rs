use eframe::egui;

use crate::pipewire_backend::{PwNode, PwStateExt};

use super::PipeMeeterApp;

impl PipeMeeterApp {
    fn draw_node_picker_list(
        ui: &mut egui::Ui,
        nodes: &[PwNode],
        draft_represented_node_name: &mut String,
        id_salt: &str,
    ) {
        egui::ScrollArea::vertical()
            .id_salt(id_salt)
            .max_height(160.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if nodes.is_empty() {
                    ui.label("No audio nodes found.");
                    return;
                }

                for node in nodes {
                    let line = if let Some(description) = node.description.as_deref() {
                        format!("#{} {} ({})", node.id, node.name, description)
                    } else {
                        format!("#{} {}", node.id, node.name)
                    };

                    if ui
                        .selectable_label(*draft_represented_node_name == node.name, line)
                        .clicked()
                    {
                        *draft_represented_node_name = node.name.clone();
                    }
                }
            });
    }

    pub(super) fn show_edit_dialog(&mut self, ctx: &egui::Context) {
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
        let mut new_strip_name = String::new();
        let mut new_represented_node_name = String::new();

        let filter = dialog.target.node_filter();
        let (filtered_nodes, all_nodes) = {
            let objects = self.backend.objects.lock().unwrap();
            let mut all_nodes = objects.nodes().cloned().collect::<Vec<_>>();
            all_nodes.sort_by_key(|node| node.id);

            let filtered_nodes = all_nodes
                .iter()
                .filter(|node| node.category == filter)
                .cloned()
                .collect::<Vec<_>>();

            (filtered_nodes, all_nodes)
        };

        egui::Window::new("Configure Strip")
            .collapsible(false)
            .resizable(false)
            .open(&mut is_open)
            .show(ctx, |ui| {
                ui.label("Strip name");
                ui.text_edit_singleline(&mut dialog.draft_strip_name);
                ui.add_space(8.0);

                ui.label("Represented PipeWire node name");
                ui.text_edit_singleline(&mut dialog.draft_represented_node_name);
                ui.add_space(8.0);

                let has_represented_name = !dialog.draft_represented_node_name.trim().is_empty();

                if has_represented_name {
                    egui::CollapsingHeader::new("Available nodes")
                        .default_open(false)
                        .show(ui, |ui| {
                            Self::draw_node_picker_list(
                                ui,
                                &filtered_nodes,
                                &mut dialog.draft_represented_node_name,
                                "filtered_nodes_collapsed",
                            );
                            ui.add_space(6.0);
                            egui::CollapsingHeader::new("All nodes")
                                .default_open(false)
                                .show(ui, |ui| {
                                    Self::draw_node_picker_list(
                                        ui,
                                        &all_nodes,
                                        &mut dialog.draft_represented_node_name,
                                        "all_nodes_nested",
                                    );
                                });
                        });
                } else {
                    ui.label("Available nodes");
                    Self::draw_node_picker_list(
                        ui,
                        &filtered_nodes,
                        &mut dialog.draft_represented_node_name,
                        "filtered_nodes_visible",
                    );
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new("All nodes")
                        .default_open(false)
                        .show(ui, |ui| {
                            Self::draw_node_picker_list(
                                ui,
                                &all_nodes,
                                &mut dialog.draft_represented_node_name,
                                "all_nodes_expanded",
                            );
                        });
                }

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        new_strip_name = dialog.draft_strip_name.clone();
                        new_represented_node_name = dialog.draft_represented_node_name.clone();
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

        if !is_open && action.is_none() {
            action = Some(DialogAction::Cancel);
        }

        match action {
            Some(DialogAction::Save) => {
                self.apply_dialog_update(dialog.target, new_strip_name, new_represented_node_name);
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
