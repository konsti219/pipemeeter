use std::collections::{HashMap, HashSet};

use glob::Pattern;

use crate::config::{NodeMatchProperty, NodeMatchRequirement, StripConfig};
use crate::pipewire_backend::{PwNode, PwNodeCategory, PwObject, PwStateExt};

use super::{PipeMeeterApp, ResolvedNodeEntry, ResolvedNodeInfo, StripTarget};

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

fn strip_nodes_to_resolved(nodes: &[&PwNode]) -> ResolvedNodeInfo {
    ResolvedNodeInfo {
        nodes: nodes
            .iter()
            .map(|node| ResolvedNodeEntry {
                id: node.id,
                display_text: node_display_text(node),
            })
            .collect(),
    }
}

fn resolve_group(
    resolved: &mut HashMap<StripTarget, ResolvedNodeInfo>,
    nodes: &[&PwNode],
    strips: &[StripConfig],
    category: PwNodeCategory,
    enable_virtual_fallback: bool,
    max_matches_per_strip: Option<usize>,
) {
    let mut assigned_nodes = HashSet::<u32>::new();
    let mut ordered_strips = strips.iter().enumerate().collect::<Vec<_>>();

    if enable_virtual_fallback {
        // Apply explicit match requirements first, then let fallback strips take leftovers.
        ordered_strips.sort_by_key(|(_, strip)| strip.represented_node_requirements.is_empty());
    }

    for (index, strip) in ordered_strips {
        let target = StripTarget::new(index, category);
        let requirements = strip.represented_node_requirements.as_slice();

        let mut matched_nodes = if requirements.is_empty() {
            if !enable_virtual_fallback {
                continue;
            }

            nodes
                .iter()
                .copied()
                .filter(|node| node.category == target.category)
                .filter(|node| !assigned_nodes.contains(&node.id))
                .collect::<Vec<_>>()
        } else {
            nodes
                .iter()
                .copied()
                .filter(|node| !assigned_nodes.contains(&node.id))
                .filter(|node| {
                    requirements
                        .iter()
                        .all(|requirement| requirement_matches_node(node, requirement))
                })
                .collect::<Vec<_>>()
        };

        if let Some(max_matches) = max_matches_per_strip {
            matched_nodes.truncate(max_matches);
        }

        if matched_nodes.is_empty() {
            continue;
        }

        for node in &matched_nodes {
            assigned_nodes.insert(node.id);
        }

        resolved.insert(target, strip_nodes_to_resolved(&matched_nodes));
    }
}

fn format_resolved_title(resolved: &ResolvedNodeInfo) -> (String, Option<String>) {
    match resolved.nodes.as_slice() {
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
    pub(super) fn resolved_node_ids(&self, target: StripTarget) -> Vec<u32> {
        self.resolved_nodes
            .get(&target)
            .map(|node| node.nodes.iter().map(|entry| entry.id).collect())
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
        let first_node = resolved.nodes.first()?;
        let objects = self.backend.objects.lock().unwrap();
        let PwObject::Node(node) = objects.get(&first_node.id)? else {
            return None;
        };

        Some(super::volume::pipewire_stereo_to_human_slider(node.volume))
    }

    pub(super) fn refresh_resolved_nodes(&mut self) {
        let objects = self.backend.objects.lock().unwrap();

        let mut nodes = objects.nodes().collect::<Vec<_>>();
        nodes.sort_by_key(|node| node.id);

        let mut resolved_nodes = HashMap::new();

        resolve_group(
            &mut resolved_nodes,
            &nodes,
            &self.config.physical_inputs,
            PwNodeCategory::InputDevice,
            false,
            Some(1),
        );

        resolve_group(
            &mut resolved_nodes,
            &nodes,
            &self.config.virtual_inputs,
            PwNodeCategory::PlaybackStream,
            true,
            None,
        );

        resolve_group(
            &mut resolved_nodes,
            &nodes,
            &self.config.physical_outputs,
            PwNodeCategory::OutputDevice,
            false,
            Some(1),
        );

        resolve_group(
            &mut resolved_nodes,
            &nodes,
            &self.config.virtual_outputs,
            PwNodeCategory::RecordingStream,
            true,
            None,
        );

        self.resolved_nodes = resolved_nodes;
    }
}
