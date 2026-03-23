use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::config::AppConfig;
use crate::pipewire_backend::{DesiredNodeLink, PwNodeCategory, PwObject, VIRTUAL_DEVICE_PREFIX};

use super::node_resolution::resolve_nodes_for_config;
use super::{PipeMeeterApp, StripTarget};

const ROUTING_WORKER_INTERVAL: Duration = Duration::from_millis(250);

pub(super) fn spawn_routing_worker(
    backend: crate::pipewire_backend::PipewireBackendClient,
    shared_config: Arc<Mutex<AppConfig>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let config = shared_config.lock().unwrap().clone();
            let objects = backend.objects.lock().unwrap().clone();

            if !graph_bootstrap_ready(&objects) {
                thread::sleep(ROUTING_WORKER_INTERVAL);
                continue;
            }

            let resolved_nodes = resolve_nodes_for_config(&config, &objects);

            backend
                .sync_managed_virtual_devices(managed_virtual_strip_names(&config))
                .unwrap();

            backend
                .sync_virtual_meters(meter_target_node_names(&config, &resolved_nodes, &objects))
                .unwrap();

            sync_virtual_input_combined_volumes(&backend, &config, &objects);

            let desired_links = desired_routing_links(&config, &resolved_nodes, &objects);
            backend.sync_routing(desired_links).unwrap();

            thread::sleep(ROUTING_WORKER_INTERVAL);
        }
    })
}

fn managed_virtual_strip_names(config: &AppConfig) -> Vec<String> {
    let mut names = Vec::new();

    for i in 0..config.virtual_inputs.len() {
        names.push(virtual_input_combined_name(i));
    }

    for i in 0..config.virtual_outputs.len() {
        names.push(virtual_output_combined_name(i));
    }

    names
}

fn managed_node_id(objects: &HashMap<u32, PwObject>, managed_name: &str) -> Option<u32> {
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

fn resolved_ids_for(
    resolved_nodes: &HashMap<StripTarget, Vec<super::ResolvedNodeEntry>>,
    target: StripTarget,
) -> Vec<u32> {
    resolved_nodes
        .get(&target)
        .map(|node| node.iter().map(|entry| entry.id).collect())
        .unwrap_or_default()
}

fn meter_target_node_names(
    config: &AppConfig,
    resolved_nodes: &HashMap<StripTarget, Vec<super::ResolvedNodeEntry>>,
    objects: &HashMap<u32, PwObject>,
) -> Vec<String> {
    let id_to_name = objects
        .values()
        .filter_map(|obj| {
            let PwObject::Node(node) = obj else {
                return None;
            };
            Some((node.id, node.name.clone()))
        })
        .collect::<HashMap<_, _>>();

    let mut names = HashSet::new();

    for name in managed_virtual_strip_names(config) {
        names.insert(name);
    }

    for index in 0..config.physical_inputs.len() {
        let target = StripTarget::new(index, PwNodeCategory::InputDevice);
        for node_id in resolved_ids_for(resolved_nodes, target) {
            if let Some(name) = id_to_name.get(&node_id) {
                names.insert(name.clone());
            }
        }
    }

    for index in 0..config.physical_outputs.len() {
        let target = StripTarget::new(index, PwNodeCategory::OutputDevice);
        for node_id in resolved_ids_for(resolved_nodes, target) {
            if let Some(name) = id_to_name.get(&node_id) {
                names.insert(name.clone());
            }
        }
    }

    names.into_iter().collect()
}

fn output_target_nodes_for_route(
    config: &AppConfig,
    resolved_nodes: &HashMap<StripTarget, Vec<super::ResolvedNodeEntry>>,
    route_index: usize,
    virtual_output_combined_ids: &[Option<u32>],
) -> Vec<u32> {
    if route_index < config.physical_outputs.len() {
        let target = StripTarget::new(route_index, PwNodeCategory::OutputDevice);
        resolved_ids_for(resolved_nodes, target)
    } else {
        let virtual_index = route_index - config.physical_outputs.len();
        virtual_output_combined_ids
            .get(virtual_index)
            .copied()
            .flatten()
            .into_iter()
            .collect()
    }
}

fn desired_routing_links(
    config: &AppConfig,
    resolved_nodes: &HashMap<StripTarget, Vec<super::ResolvedNodeEntry>>,
    objects: &HashMap<u32, PwObject>,
) -> Vec<DesiredNodeLink> {
    let virtual_input_combined_ids = (0..config.virtual_inputs.len())
        .map(|index| managed_node_id(objects, &virtual_input_combined_name(index)))
        .collect::<Vec<_>>();
    let virtual_output_combined_ids = (0..config.virtual_outputs.len())
        .map(|index| managed_node_id(objects, &virtual_output_combined_name(index)))
        .collect::<Vec<_>>();

    let mut desired = HashSet::new();

    for (index, combined_node_id) in virtual_input_combined_ids.iter().enumerate() {
        let Some(combined_node_id) = combined_node_id else {
            continue;
        };

        let source_target = StripTarget::new(index, PwNodeCategory::PlaybackStream);
        for source_node_id in resolved_ids_for(resolved_nodes, source_target) {
            desired.insert(DesiredNodeLink {
                output_node: source_node_id,
                input_node: *combined_node_id,
            });
        }
    }

    for (index, strip) in config.physical_inputs.iter().enumerate() {
        let source_target = StripTarget::new(index, PwNodeCategory::InputDevice);
        let source_nodes = resolved_ids_for(resolved_nodes, source_target);

        for (route_index, enabled) in strip.routes_to_outputs.iter().copied().enumerate() {
            if !enabled {
                continue;
            }

            let target_nodes = output_target_nodes_for_route(
                config,
                resolved_nodes,
                route_index,
                &virtual_output_combined_ids,
            );
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

    for (index, strip) in config.virtual_inputs.iter().enumerate() {
        let Some(source_node) = virtual_input_combined_ids.get(index).copied().flatten() else {
            continue;
        };

        for (route_index, enabled) in strip.routes_to_outputs.iter().copied().enumerate() {
            if !enabled {
                continue;
            }

            let target_nodes = output_target_nodes_for_route(
                config,
                resolved_nodes,
                route_index,
                &virtual_output_combined_ids,
            );
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
        for sink_node_id in resolved_ids_for(resolved_nodes, sink_target) {
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

fn sync_virtual_input_combined_volumes(
    backend: &crate::pipewire_backend::PipewireBackendClient,
    config: &AppConfig,
    objects: &HashMap<u32, PwObject>,
) {
    for index in 0..config.virtual_inputs.len() {
        let Some(combined_node_id) = managed_node_id(objects, &virtual_input_combined_name(index))
        else {
            continue;
        };

        let slider = config
            .virtual_inputs
            .get(index)
            .map(|strip| strip.volume)
            .unwrap_or(1.0);
        let desired_linear = super::volume::human_slider_to_pipewire_linear(slider);

        let should_update = match objects.get(&combined_node_id) {
            Some(PwObject::Node(node)) => {
                (node.volume[0] - desired_linear).abs() > 0.01
                    || (node.volume[1] - desired_linear).abs() > 0.01
            }
            _ => false,
        };

        if should_update {
            backend
                .set_node_volume(combined_node_id, desired_linear)
                .unwrap();
        }
    }
}

fn virtual_input_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vin-{}", index + 1)
}

fn virtual_output_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vout-{}", index + 1)
}

fn graph_bootstrap_ready(objects: &HashMap<u32, PwObject>) -> bool {
    let app_name = env!("CARGO_PKG_NAME").to_ascii_lowercase();

    let mut has_adapter_factory = false;
    let mut has_link_factory = false;
    let mut has_app_client = false;

    for obj in objects.values() {
        match obj {
            PwObject::Factory(factory) => {
                if factory.type_name == "PipeWire:Interface:Node"
                    && factory.name.starts_with("adapter")
                {
                    has_adapter_factory = true;
                }

                if factory.type_name == "PipeWire:Interface:Link" {
                    has_link_factory = true;
                }
            }
            PwObject::Client(client) => {
                let candidate = client.application_name.to_ascii_lowercase();
                if candidate == app_name || candidate.contains(&app_name) {
                    has_app_client = true;
                }
            }
            _ => {}
        }
    }

    has_adapter_factory && has_link_factory && has_app_client
}

impl PipeMeeterApp {
    pub(super) fn virtual_input_combined_node_id(&self, index: usize) -> Option<u32> {
        self.managed_node_id(&virtual_input_combined_name(index))
    }

    pub(super) fn virtual_output_combined_node_id(&self, index: usize) -> Option<u32> {
        self.managed_node_id(&virtual_output_combined_name(index))
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

    pub(super) fn apply_virtual_input_slider_volume(&mut self, index: usize, slider: f32) {
        let Some(combined_node_id) = self.virtual_input_combined_node_id(index) else {
            return;
        };
        let linear = super::volume::human_slider_to_pipewire_linear(slider);
        self.backend
            .set_node_volume(combined_node_id, linear)
            .unwrap();
    }
}
