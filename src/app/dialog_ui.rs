use eframe::egui;
use glob::Pattern;

use crate::config::{NodeMatchProperty, NodeMatchRequirement};
use crate::pipewire_backend::{PwNode, PwStateExt};

use super::PipeMeeterApp;

impl PipeMeeterApp {
    fn node_match_value<'a>(
        node: &'a PwNode,
        match_property: NodeMatchProperty,
    ) -> Option<&'a str> {
        match match_property {
            NodeMatchProperty::Name => {
                let value = node.name.trim();
                if value.is_empty() { None } else { Some(value) }
            }
            NodeMatchProperty::Description => node
                .description
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            NodeMatchProperty::MediaName => node
                .media_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            NodeMatchProperty::ProcessBinary => node
                .process_binary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        }
    }

    fn draw_node_picker_list(
        ui: &mut egui::Ui,
        nodes: &[PwNode],
        mut selected_requirement: Option<&mut NodeMatchRequirement>,
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
                    let current_property = selected_requirement
                        .as_ref()
                        .map(|requirement| requirement.match_property);
                    let current_pattern = selected_requirement
                        .as_ref()
                        .map(|requirement| requirement.pattern.clone())
                        .unwrap_or_default();
                    let selected_value = current_property
                        .and_then(|property| Self::node_match_value(node, property));
                    let line = if let Some(value) = selected_value {
                        format!("#{} {} [name: {}]", node.id, value, node.name)
                    } else {
                        let label = current_property
                            .map(|property| property.label())
                            .unwrap_or("property");
                        format!("#{} <missing {}> [name: {}]", node.id, label, node.name)
                    };

                    let is_selected = selected_value == Some(current_pattern.as_str());

                    if ui.selectable_label(is_selected, line).clicked() {
                        if let (Some(value), Some(requirement)) =
                            (selected_value, selected_requirement.as_deref_mut())
                        {
                            requirement.pattern = value.to_owned();
                        }
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
        let mut new_represented_node_requirements = Vec::new();
        let mut new_match_only_category = dialog.draft_match_only_category;
        let fallback_only = self.is_default_strip(dialog.target);

        let filter = dialog.target.category;
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

                if fallback_only {
                    ui.small("Fallback strip: only the name can be changed.");
                    ui.add_space(10.0);
                } else {
                    ui.label("Node match requirements (all must match)");

                    let mut remove_index = None;
                    for (index, requirement) in dialog
                        .draft_represented_node_requirements
                        .iter_mut()
                        .enumerate()
                    {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}.", index + 1));
                            egui::ComboBox::from_id_salt(format!(
                                "represented_node_match_{}",
                                index
                            ))
                            .selected_text(requirement.match_property.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut requirement.match_property,
                                    NodeMatchProperty::Name,
                                    NodeMatchProperty::Name.label(),
                                );
                                ui.selectable_value(
                                    &mut requirement.match_property,
                                    NodeMatchProperty::Description,
                                    NodeMatchProperty::Description.label(),
                                );
                                ui.selectable_value(
                                    &mut requirement.match_property,
                                    NodeMatchProperty::MediaName,
                                    NodeMatchProperty::MediaName.label(),
                                );
                                ui.selectable_value(
                                    &mut requirement.match_property,
                                    NodeMatchProperty::ProcessBinary,
                                    NodeMatchProperty::ProcessBinary.label(),
                                );
                            });

                            ui.text_edit_singleline(&mut requirement.pattern);

                            let invalid = {
                                let trimmed = requirement.pattern.trim();
                                !trimmed.is_empty() && Pattern::new(trimmed).is_err()
                            };
                            if invalid {
                                ui.colored_label(egui::Color32::RED, "invalid glob");
                            }

                            if ui.small_button("Pick").clicked() {
                                dialog.selected_requirement_index = index;
                            }

                            if ui.small_button("- Remove").clicked() {
                                remove_index = Some(index);
                            }
                        });
                    }

                    if let Some(index) = remove_index {
                        dialog.draft_represented_node_requirements.remove(index);
                        if dialog.selected_requirement_index
                            >= dialog.draft_represented_node_requirements.len()
                        {
                            dialog.selected_requirement_index = dialog
                                .draft_represented_node_requirements
                                .len()
                                .saturating_sub(1);
                        }
                    }

                    if ui.button("+ Add requirement").clicked() {
                        dialog
                            .draft_represented_node_requirements
                            .push(NodeMatchRequirement::new(
                                String::new(),
                                NodeMatchProperty::Name,
                            ));
                        dialog.selected_requirement_index = dialog
                            .draft_represented_node_requirements
                            .len()
                            .saturating_sub(1);
                    }

                    ui.checkbox(
                        &mut dialog.draft_match_only_category,
                        "Only match nodes from this strip category",
                    );
                    ui.add_space(8.0);

                    let selected_requirement = dialog
                        .draft_represented_node_requirements
                        .get_mut(dialog.selected_requirement_index);

                    if selected_requirement.is_none() {
                        ui.small("Add and select a requirement to use the node picker.");
                    }

                    ui.label("Available category nodes");
                    Self::draw_node_picker_list(
                        ui,
                        &filtered_nodes,
                        selected_requirement,
                        "filtered_nodes_visible",
                    );
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new("All nodes")
                        .default_open(false)
                        .show(ui, |ui| {
                            let selected_requirement = dialog
                                .draft_represented_node_requirements
                                .get_mut(dialog.selected_requirement_index);
                            Self::draw_node_picker_list(
                                ui,
                                &all_nodes,
                                selected_requirement,
                                "all_nodes_expanded",
                            );
                        });

                    ui.add_space(10.0);
                }

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        new_strip_name = dialog.draft_strip_name.clone();
                        new_represented_node_requirements =
                            dialog.draft_represented_node_requirements.clone();
                        new_match_only_category = dialog.draft_match_only_category;
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
                self.apply_dialog_update(
                    dialog.target,
                    new_strip_name,
                    new_represented_node_requirements,
                    new_match_only_category,
                );
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
