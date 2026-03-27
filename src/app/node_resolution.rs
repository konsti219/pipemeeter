use crate::config::AppConfig;
use crate::pipewire_backend::{PwNode, PwNodeCategory, PwObject, PwState};
use crate::volume::pipewire_linear_to_slider;

use super::types::ResolvedNodeEntry;
use super::{PipeMeeterApp, StripTarget};

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

fn format_resolved_title(resolved: &[ResolvedNodeEntry]) -> (String, Option<String>) {
    match resolved {
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
    pub(super) fn resolved_node_ids_from_config(
        config: &AppConfig,
        target: StripTarget,
    ) -> Vec<u32> {
        match target.category {
            PwNodeCategory::InputDevice => config
                .physical_inputs
                .get(target.index)
                .map(|strip| strip.resolved_nodes.clone()),
            PwNodeCategory::PlaybackStream => config
                .virtual_inputs
                .get(target.index)
                .map(|strip| strip.resolved_nodes.clone()),
            PwNodeCategory::OutputDevice => config
                .physical_outputs
                .get(target.index)
                .map(|strip| strip.resolved_nodes.clone()),
            PwNodeCategory::RecordingStream => config
                .virtual_outputs
                .get(target.index)
                .map(|strip| strip.resolved_nodes.clone()),
            _ => None,
        }
        .unwrap_or_default()
    }

    pub(super) fn resolved_node_title_from_state(
        config: &AppConfig,
        objects: &PwState,
        target: StripTarget,
    ) -> Option<(String, Option<String>)> {
        let resolved_ids = Self::resolved_node_ids_from_config(config, target);
        if resolved_ids.is_empty() {
            return None;
        }

        let resolved = resolved_ids
            .iter()
            .map(|id| {
                let display_text = match objects.get(id) {
                    Some(PwObject::Node(node)) => node_display_text(node),
                    _ => "unknown".to_owned(),
                };
                ResolvedNodeEntry {
                    id: *id,
                    display_text,
                }
            })
            .collect::<Vec<_>>();

        Some(format_resolved_title(&resolved))
    }

    pub(super) fn resolved_meter_level_from_config(
        &self,
        config: &AppConfig,
        target: StripTarget,
    ) -> [f32; 2] {
        match target.category {
            PwNodeCategory::InputDevice => Self::resolved_node_ids_from_config(config, target)
                .first()
                .copied()
                .map(|id| self.backend.node_peak_meter(id))
                .unwrap_or_default(),
            PwNodeCategory::OutputDevice => {
                let slider = match target.category {
                    PwNodeCategory::OutputDevice => config
                        .physical_outputs
                        .get(target.index)
                        .map(|strip| strip.volume)
                        .unwrap_or(1.0),
                    _ => 1.0,
                };
                Self::resolved_node_ids_from_config(config, target)
                    .first()
                    .copied()
                    .map(|id| self.backend.node_peak_meter(id))
                    .map(|levels| [levels[0] * slider, levels[1] * slider])
                    .unwrap_or_default()
            }
            PwNodeCategory::PlaybackStream => self
                .virtual_input_node_id(target.index)
                .map(|id| self.backend.node_peak_meter(id))
                .unwrap_or_default(),
            PwNodeCategory::RecordingStream => self
                .virtual_output_node_id(target.index)
                .map(|id| self.backend.node_peak_meter(id))
                .unwrap_or_default(),
            _ => [0.0, 0.0],
        }
    }

    pub(super) fn refresh_resolved_nodes_with_state(
        &mut self,
        config: &mut AppConfig,
        objects: &PwState,
    ) {
        for strip in &mut config.physical_outputs {
            let Some(node_id) = strip.resolved_nodes.first().copied() else {
                continue;
            };

            let Some(PwObject::Node(node)) = objects.get(&node_id) else {
                continue;
            };

            let linear = (node.volume[0] + node.volume[1]) * 0.5;
            strip.volume = pipewire_linear_to_slider(linear);
        }
    }
}
