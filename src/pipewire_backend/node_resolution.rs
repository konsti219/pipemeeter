use std::collections::HashSet;

use glob::Pattern;

use crate::config::{AppConfig, NodeMatchProperty, NodeMatchRequirement, StripConfig};

use super::*;

pub fn managed_virtual_strip_names(config: &AppConfig) -> Vec<String> {
    let mut names = Vec::new();

    for i in 0..config.virtual_inputs.len() {
        names.push(virtual_input_combined_name(i));
    }

    for i in 0..config.virtual_outputs.len() {
        names.push(virtual_output_combined_name(i));
    }

    names
}

pub fn managed_node_id(state: &PwState, managed_name: &str) -> Option<u32> {
    state.values().find_map(|obj| {
        let PwObject::Node(node) = obj else {
            return None;
        };

        if node.name == managed_name {
            Some(node.id)
        } else {
            None
        }
    })
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

fn resolve_physical_strips(
    assigned_nodes: &mut HashSet<u32>,
    nodes: &[&PwNode],
    strips: &mut [StripConfig],
    category: PwNodeCategory,
) {
    // let mut out = vec![Vec::new(); strips.len()];

    for strip in strips {
        strip.resolved_nodes.clear();

        let requirements = strip.requirements.as_slice();
        if requirements.is_empty() {
            continue;
        }

        let mut candidates = nodes
            .iter()
            .copied()
            .filter(|node| !assigned_nodes.contains(&node.id))
            .filter(|node| {
                requirements
                    .iter()
                    .all(|requirement| requirement_matches_node(node, requirement))
            });

        if let Some(node) = candidates.find(|node| node.category == category) {
            assigned_nodes.insert(node.id);
            strip.resolved_nodes.push(node.id);
            continue;
        }

        if strip.match_only_category {
            continue;
        }

        if let Some(node) = candidates.next() {
            assigned_nodes.insert(node.id);
            strip.resolved_nodes.push(node.id);
        }
    }
}

fn resolve_virtual_strips(
    assigned_nodes: &mut HashSet<u32>,
    nodes: &[&PwNode],
    strips: &mut [StripConfig],
    category: PwNodeCategory,
) {
    // First resolve strips with any requirements
    for strip in strips
        .iter_mut()
        .filter(|strip| !strip.requirements.is_empty())
    {
        let ids = nodes
            .iter()
            .copied()
            .filter(|node| !assigned_nodes.contains(&node.id))
            .filter(|node| {
                strip
                    .requirements
                    .iter()
                    .all(|requirement| requirement_matches_node(node, requirement))
            })
            .filter(|node| !strip.match_only_category || node.category == category)
            .map(|node| node.id)
            .collect::<Vec<_>>();

        for id in &ids {
            assigned_nodes.insert(*id);
        }

        strip.resolved_nodes = ids;
    }

    // Then strips without requirements
    for strip in strips
        .iter_mut()
        .filter(|strip| strip.requirements.is_empty())
    {
        let ids = nodes
            .iter()
            .copied()
            .filter(|node| !assigned_nodes.contains(&node.id))
            .filter(|node| node.category == category)
            .map(|node| node.id)
            .collect::<Vec<_>>();

        for id in &ids {
            assigned_nodes.insert(*id);
        }

        strip.resolved_nodes = ids;
    }
}

pub fn resolve_nodes(config: &mut AppConfig, state: &PwState) {
    let mut nodes = state
        .nodes()
        .filter(|node| !node.category.is_pipemeeter())
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| node.id);

    let mut assigned_node_ids = HashSet::new();

    resolve_physical_strips(
        &mut assigned_node_ids,
        &nodes,
        &mut config.physical_inputs,
        PwNodeCategory::InputDevice,
    );

    resolve_physical_strips(
        &mut assigned_node_ids,
        &nodes,
        &mut config.physical_outputs,
        PwNodeCategory::OutputDevice,
    );

    resolve_virtual_strips(
        &mut assigned_node_ids,
        &nodes,
        &mut config.virtual_outputs,
        PwNodeCategory::RecordingStream,
    );

    resolve_virtual_strips(
        &mut assigned_node_ids,
        &nodes,
        &mut config.virtual_inputs,
        PwNodeCategory::PlaybackStream,
    );
}
