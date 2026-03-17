use std::collections::{HashMap, HashSet};

use glob::Pattern;

use crate::config::{NodeMatchProperty, NodeMatchRequirement, StripConfig};
use crate::pipewire_backend::{PwNode, PwNodeCategory, PwObject, PwStateExt};

use super::{PipeMeeterApp, ResolvedNodeEntry, StripTarget};

fn node_display_text(node: &PwNode) -> String {
    node.description
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            node.media_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            let name = node.name.trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_owned())
            }
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn node_match_value<'a>(node: &'a PwNode, match_property: NodeMatchProperty) -> Option<&'a str> {
    let val = match match_property {
        NodeMatchProperty::Name => Some(node.name.as_str()),
        NodeMatchProperty::Description => node.description.as_deref(),
        NodeMatchProperty::MediaName => node.media_name.as_deref(),
        NodeMatchProperty::ProcessBinary => node.process_binary.as_deref(),
    };
    val.map(str::trim).filter(|value| !value.is_empty())
}

fn requirement_matches_node(node: &PwNode, requirement: &NodeMatchRequirement) -> bool {
    let value = match node_match_value(node, requirement.match_property) {
        Some(value) => value,
        None => return false,
    };

    let pattern = requirement.pattern.trim();
    if pattern.is_empty() {
        return false;
    }

    match Pattern::new(pattern) {
        Ok(glob_pattern) => glob_pattern.matches(value),
        Err(_) => false,
    }
}

fn strip_nodes_to_resolved<'a>(
    nodes: impl IntoIterator<Item = &'a &'a PwNode>,
) -> Vec<ResolvedNodeEntry> {
    nodes
        .into_iter()
        .map(|node| ResolvedNodeEntry {
            id: node.id,
            display_text: node_display_text(node),
        })
        .collect()
}

fn resolve_physical(
    resolved: &mut HashMap<StripTarget, Vec<ResolvedNodeEntry>>,
    nodes: &[&PwNode],
    strips: &[StripConfig],
    category: PwNodeCategory,
) {
    let mut assigned_nodes = HashSet::<u32>::new();

    for (index, strip) in strips.iter().enumerate() {
        let target = StripTarget::new(index, category);
        let requirements = strip.requirements.as_slice();
        if requirements.is_empty() {
            continue;
        }

        let mut candidates = nodes
            .into_iter()
            .copied()
            .filter(|node| !assigned_nodes.contains(&node.id))
            .filter(|node| {
                requirements
                    .iter()
                    .all(|requirement| requirement_matches_node(node, requirement))
            });

        // First check if there is a node that matches the requirements and category
        if let Some(res) = candidates.find(|node| node.category == category) {
            assigned_nodes.insert(res.id);
            resolved.insert(target, strip_nodes_to_resolved(&[res]));
            continue;
        }

        if let Some(res) = candidates.next() {
            assigned_nodes.insert(res.id);
            resolved.insert(target, strip_nodes_to_resolved(&[res]));
        }
    }
}

fn resolve_virtual(
    resolved: &mut HashMap<StripTarget, Vec<ResolvedNodeEntry>>,
    nodes: &[&PwNode],
    strips: &[StripConfig],
    category: PwNodeCategory,
) {
    let mut assigned_nodes = HashSet::<u32>::new();

    // First iterate over strips with requirements
    for (index, strip) in strips
        .iter()
        .enumerate()
        .filter(|(_, strip)| !strip.requirements.is_empty())
    {
        let target = StripTarget::new(index, category);

        let nodes = nodes
            .iter()
            .copied()
            .filter(|node| !assigned_nodes.contains(&node.id))
            .filter(|node| {
                strip
                    .requirements
                    .iter()
                    .all(|requirement| requirement_matches_node(node, requirement))
            })
            .collect::<Vec<_>>();

        for node in &nodes {
            assigned_nodes.insert(node.id);
        }

        resolved.insert(target, strip_nodes_to_resolved(&nodes));
    }

    // Then iterate over strips without requirements and assign any remaining nodes
    for (index, _strip) in strips
        .iter()
        .enumerate()
        .filter(|(_, strip)| strip.requirements.is_empty())
    {
        let target = StripTarget::new(index, category);

        let nodes = nodes
            .iter()
            .copied()
            .filter(|node| !assigned_nodes.contains(&node.id))
            .filter(|node| node.category == category)
            .collect::<Vec<_>>();

        for node in &nodes {
            assigned_nodes.insert(node.id);
        }

        resolved.insert(target, strip_nodes_to_resolved(&nodes));
    }
}

fn format_resolved_title(resolved: &Vec<ResolvedNodeEntry>) -> (String, Option<String>) {
    match resolved.as_slice() {
        [] => (String::new(), None),
        [single] => (format!("#{} {}", single.id, single.display_text), None),
        [first, second] => (
            format!("#{} {}", first.id, first.display_text),
            Some(format!("#{} {}", second.id, second.display_text)),
        ),
        [first, rest @ ..] => (
            format!("#{} {}", first.id, first.display_text),
            Some(
                rest.iter()
                    .map(|entry| format!("#{}", entry.id))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ),
    }
}

impl PipeMeeterApp {
    fn node_volume_slider_value(&self, node_id: u32) -> Option<f32> {
        let objects = self.backend.objects.lock().unwrap();
        let PwObject::Node(node) = objects.get(&node_id)? else {
            return None;
        };

        Some(super::volume::pipewire_stereo_to_human_slider(node.volume))
    }

    pub(super) fn resolved_node_ids(&self, target: StripTarget) -> Vec<u32> {
        self.resolved_nodes
            .get(&target)
            .map(|node| node.iter().map(|entry| entry.id).collect())
            .unwrap_or_default()
    }

    pub(super) fn resolved_node_title(
        &self,
        target: StripTarget,
    ) -> Option<(String, Option<String>)> {
        self.resolved_nodes.get(&target).map(format_resolved_title)
    }

    pub(super) fn resolved_volume_slider_value(&self, target: StripTarget) -> Option<f32> {
        let resolved = self.resolved_nodes.get(&target)?;
        let first_node = resolved.first()?;
        self.node_volume_slider_value(first_node.id)
    }

    pub(super) fn resolved_meter_level(&self, target: StripTarget) -> Option<f32> {
        match target.category {
            PwNodeCategory::PlaybackStream => self
                .virtual_input_combined_node_id(target.index)
                .and_then(|id| self.backend.node_peak_meter(id)),
            PwNodeCategory::RecordingStream => self
                .virtual_output_combined_node_id(target.index)
                .and_then(|id| self.backend.node_peak_meter(id)),
            _ => None,
        }
    }

    pub(super) fn refresh_resolved_nodes(&mut self) {
        let objects = self.backend.objects.lock().unwrap();

        let mut nodes = objects
            .nodes()
            .filter(|node| node.category != PwNodeCategory::Pipemeeter)
            .collect::<Vec<_>>();
        nodes.sort_by_key(|node| node.id);

        let mut resolved_nodes = HashMap::new();

        resolve_physical(
            &mut resolved_nodes,
            &nodes,
            &self.config.physical_inputs,
            PwNodeCategory::InputDevice,
        );

        resolve_virtual(
            &mut resolved_nodes,
            &nodes,
            &self.config.virtual_inputs,
            PwNodeCategory::PlaybackStream,
        );

        resolve_physical(
            &mut resolved_nodes,
            &nodes,
            &self.config.physical_outputs,
            PwNodeCategory::OutputDevice,
        );

        resolve_virtual(
            &mut resolved_nodes,
            &nodes,
            &self.config.virtual_outputs,
            PwNodeCategory::RecordingStream,
        );

        self.resolved_nodes = resolved_nodes;
    }
}
