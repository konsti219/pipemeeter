use std::collections::HashMap;

use crate::config::NodeMatchProperty;
use crate::pipewire_backend::{PwNode, PwObject, PwStateExt};

use super::{Group, PipeMeeterApp, ResolvedNodeInfo, StripTarget};

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
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
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

fn resolve_group<'a, I, F>(
    resolved: &mut HashMap<StripTarget, ResolvedNodeInfo>,
    nodes: &[&PwNode],
    strips: I,
    make_target: F,
) where
    I: Iterator<Item = (usize, &'a str, NodeMatchProperty)>,
    F: Fn(usize) -> StripTarget,
{
    for (index, represented_node_name, represented_node_match) in strips {
        let requested_name = represented_node_name.trim();
        if requested_name.is_empty() {
            continue;
        }

        // First try to find a node that matches both the selected property and category, then fall back to the selected property only.
        let target = make_target(index);
        let node = nodes
            .iter()
            .find(|node| {
                node_match_value(node, represented_node_match) == Some(requested_name)
                    && node.category == target.node_filter()
            })
            .or_else(|| {
                nodes
                    .iter()
                    .find(|node| node_match_value(node, represented_node_match) == Some(requested_name))
            });

        if let Some(node) = node {
            resolved.insert(
                make_target(index),
                ResolvedNodeInfo {
                    id: node.id,
                    display_text: node_display_text(node),
                },
            );
        }
    }
}

impl PipeMeeterApp {
    pub(super) fn resolved_node_id(&self, target: StripTarget) -> Option<u32> {
        self.resolved_nodes.get(&target).map(|node| node.id)
    }

    pub(super) fn resolved_node_title(&self, target: StripTarget) -> Option<String> {
        self.resolved_nodes
            .get(&target)
            .map(|node| format!("#{} {}", node.id, node.display_text))
    }

    pub(super) fn resolved_volume_slider_value(&self, target: StripTarget) -> Option<f32> {
        let resolved = self.resolved_nodes.get(&target)?;
        let objects = self.backend.objects.lock().unwrap();
        let PwObject::Node(node) = objects.get(&resolved.id)? else {
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
            self.config
                .physical_inputs
                .iter()
                .enumerate()
                .map(|(idx, strip)| {
                    (
                        idx,
                        strip.represented_node_name.as_str(),
                        strip.represented_node_match,
                    )
                }),
            |index| StripTarget::Input {
                group: Group::Physical,
                index,
            },
        );

        resolve_group(
            &mut resolved_nodes,
            &nodes,
            self.config
                .virtual_inputs
                .iter()
                .enumerate()
                .map(|(idx, strip)| {
                    (
                        idx,
                        strip.represented_node_name.as_str(),
                        strip.represented_node_match,
                    )
                }),
            |index| StripTarget::Input {
                group: Group::Virtual,
                index,
            },
        );

        resolve_group(
            &mut resolved_nodes,
            &nodes,
            self.config
                .physical_outputs
                .iter()
                .enumerate()
                .map(|(idx, strip)| {
                    (
                        idx,
                        strip.represented_node_name.as_str(),
                        strip.represented_node_match,
                    )
                }),
            |index| StripTarget::Output {
                group: Group::Physical,
                index,
            },
        );

        resolve_group(
            &mut resolved_nodes,
            &nodes,
            self.config
                .virtual_outputs
                .iter()
                .enumerate()
                .map(|(idx, strip)| {
                    (
                        idx,
                        strip.represented_node_name.as_str(),
                        strip.represented_node_match,
                    )
                }),
            |index| StripTarget::Output {
                group: Group::Virtual,
                index,
            },
        );

        self.resolved_nodes = resolved_nodes;
    }
}
