use std::collections::HashSet;
use std::time::{Duration, Instant};

use log::error;

use crate::pipewire_backend::{DesiredNodeLink, PwNodeCategory, PwObject, VIRTUAL_DEVICE_PREFIX};

use super::{PipeMeeterApp, StripTarget};

const ROUTING_SYNC_INTERVAL: Duration = Duration::from_millis(250);

fn virtual_input_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vin-{}", index + 1)
}

fn virtual_output_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vout-{}", index + 1)
}

impl PipeMeeterApp {
    fn managed_virtual_strip_names(&self) -> Vec<String> {
        let mut names = Vec::new();

        for i in 0..self.config.virtual_inputs.len() {
            names.push(virtual_input_combined_name(i));
        }

        for i in 0..self.config.virtual_outputs.len() {
            names.push(virtual_output_combined_name(i));
        }

        names
    }

    fn sync_managed_virtual_nodes(&mut self) -> anyhow::Result<()> {
        self.backend
            .sync_managed_virtual_devices(self.managed_virtual_strip_names())
    }

    fn managed_node_id(&self, managed_name: &str) -> Option<u32> {
        let objects = self.backend.objects.lock().unwrap();

        objects.values().find_map(|obj| {
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

    fn resolved_ids_for(&self, target: StripTarget) -> Vec<u32> {
        self.resolved_node_ids(target)
    }

    fn output_target_nodes_for_route(
        &self,
        route_index: usize,
        virtual_output_combined_ids: &[Option<u32>],
    ) -> Vec<u32> {
        if route_index < self.config.physical_outputs.len() {
            let target = StripTarget::new(route_index, PwNodeCategory::OutputDevice);
            self.resolved_ids_for(target)
        } else {
            let virtual_index = route_index - self.config.physical_outputs.len();
            virtual_output_combined_ids
                .get(virtual_index)
                .copied()
                .flatten()
                .into_iter()
                .collect()
        }
    }

    fn desired_routing_links(&self) -> Vec<DesiredNodeLink> {
        let virtual_input_combined_ids = (0..self.config.virtual_inputs.len())
            .map(|index| self.managed_node_id(&virtual_input_combined_name(index)))
            .collect::<Vec<_>>();
        let virtual_output_combined_ids = (0..self.config.virtual_outputs.len())
            .map(|index| self.managed_node_id(&virtual_output_combined_name(index)))
            .collect::<Vec<_>>();

        let mut desired = HashSet::new();

        for (index, combined_node_id) in virtual_input_combined_ids.iter().enumerate() {
            let Some(combined_node_id) = combined_node_id else {
                continue;
            };

            let source_target = StripTarget::new(index, PwNodeCategory::PlaybackStream);
            for source_node_id in self.resolved_ids_for(source_target) {
                desired.insert(DesiredNodeLink {
                    output_node: source_node_id,
                    input_node: *combined_node_id,
                });
            }
        }

        for (index, strip) in self.config.physical_inputs.iter().enumerate() {
            let source_target = StripTarget::new(index, PwNodeCategory::InputDevice);
            let source_nodes = self.resolved_ids_for(source_target);

            for (route_index, enabled) in strip.routes_to_outputs.iter().copied().enumerate() {
                if !enabled {
                    continue;
                }

                let target_nodes =
                    self.output_target_nodes_for_route(route_index, &virtual_output_combined_ids);
                for output_node in &source_nodes {
                    for input_node in &target_nodes {
                        if output_node == input_node {
                            continue;
                        }

                        desired.insert(DesiredNodeLink {
                            output_node: *output_node,
                            input_node: *input_node,
                        });
                    }
                }
            }
        }

        for (index, strip) in self.config.virtual_inputs.iter().enumerate() {
            let Some(source_node) = virtual_input_combined_ids.get(index).copied().flatten() else {
                continue;
            };

            for (route_index, enabled) in strip.routes_to_outputs.iter().copied().enumerate() {
                if !enabled {
                    continue;
                }

                let target_nodes =
                    self.output_target_nodes_for_route(route_index, &virtual_output_combined_ids);
                for input_node in target_nodes {
                    if source_node == input_node {
                        continue;
                    }

                    desired.insert(DesiredNodeLink {
                        output_node: source_node,
                        input_node,
                    });
                }
            }
        }

        for (index, combined_node_id) in virtual_output_combined_ids.iter().enumerate() {
            let Some(combined_node_id) = combined_node_id else {
                continue;
            };

            let sink_target = StripTarget::new(index, PwNodeCategory::RecordingStream);
            for sink_node_id in self.resolved_ids_for(sink_target) {
                if sink_node_id == *combined_node_id {
                    continue;
                }

                desired.insert(DesiredNodeLink {
                    output_node: *combined_node_id,
                    input_node: sink_node_id,
                });
            }
        }

        desired.into_iter().collect()
    }

    fn virtual_input_single_explicit_app_node(&self, index: usize) -> Option<u32> {
        let strip = self.config.virtual_inputs.get(index)?;
        if strip.requirements.is_empty() {
            return None;
        }

        let target = StripTarget::new(index, PwNodeCategory::PlaybackStream);
        let ids = self.resolved_ids_for(target);
        if ids.len() == 1 { Some(ids[0]) } else { None }
    }

    fn sync_virtual_input_combined_volumes(&mut self) {
        for index in 0..self.config.virtual_inputs.len() {
            let Some(combined_node_id) = self.managed_node_id(&virtual_input_combined_name(index))
            else {
                continue;
            };

            let desired_linear = if self.virtual_input_single_explicit_app_node(index).is_some() {
                1.0
            } else {
                let slider = self
                    .config
                    .virtual_inputs
                    .get(index)
                    .map(|strip| strip.volume)
                    .unwrap_or(1.0);
                super::volume::human_slider_to_pipewire_linear(slider)
            };

            let should_update = {
                let objects = self.backend.objects.lock().unwrap();
                match objects.get(&combined_node_id) {
                    Some(PwObject::Node(node)) => {
                        (node.volume[0] - desired_linear).abs() > 0.01
                            || (node.volume[1] - desired_linear).abs() > 0.01
                    }
                    _ => false,
                }
            };

            if should_update {
                if let Err(err) = self
                    .backend
                    .set_node_volume(combined_node_id, desired_linear)
                {
                    self.status = format!(
                        "failed to set virtual input combined volume for node #{}: {err}",
                        combined_node_id
                    );
                }
            }
        }
    }

    pub(super) fn apply_virtual_input_slider_volume(&mut self, index: usize, slider: f32) {
        if let Some(app_node_id) = self.virtual_input_single_explicit_app_node(index) {
            if let Some(combined_node_id) =
                self.managed_node_id(&virtual_input_combined_name(index))
            {
                if let Err(err) = self.backend.set_node_volume(combined_node_id, 1.0) {
                    self.status = format!(
                        "failed to reset virtual input combined volume for node #{}: {err}",
                        combined_node_id
                    );
                }
            }

            let linear = super::volume::human_slider_to_pipewire_linear(slider);
            if let Err(err) = self.backend.set_node_volume(app_node_id, linear) {
                self.status = format!(
                    "failed to set input volume for node #{}: {err}",
                    app_node_id
                );
            }
            return;
        }

        let Some(combined_node_id) = self.managed_node_id(&virtual_input_combined_name(index))
        else {
            return;
        };
        let linear = super::volume::human_slider_to_pipewire_linear(slider);
        if let Err(err) = self.backend.set_node_volume(combined_node_id, linear) {
            self.status = format!(
                "failed to set virtual input combined volume for node #{}: {err}",
                combined_node_id
            );
        }
    }

    pub(super) fn maybe_sync_audio_routing(&mut self) {
        if self
            .last_routing_sync
            .is_some_and(|last_sync| last_sync.elapsed() < ROUTING_SYNC_INTERVAL)
        {
            return;
        }

        self.last_routing_sync = Some(Instant::now());

        if let Err(err) = self.sync_managed_virtual_nodes() {
            self.status = format!("failed to sync managed virtual nodes: {err}");
            return;
        }

        self.sync_virtual_input_combined_volumes();

        let desired_links = self.desired_routing_links();
        if let Err(err) = self.backend.sync_routing(desired_links) {
            self.status = format!("failed to sync audio routing: {err}");
        }
    }
}

impl Drop for PipeMeeterApp {
    fn drop(&mut self) {
        if let Err(err) = self.backend.cleanup_managed_objects() {
            error!("failed to cleanup managed objects on app shutdown: {err}");
        }
    }
}
