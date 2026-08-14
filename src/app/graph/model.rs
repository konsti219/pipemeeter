//! Snapshot of the PipeWire audio graph, reduced to what is drawn. Everything
//! outside the audio subsystem is dropped here.

use std::collections::HashMap;

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::config::{AppConfig, StripConfig};
use crate::pipewire_backend::{
    PortDirection, PwMediaType, PwNode, PwNodeCategory, PwObject, PwPort, PwState,
    virtual_input_combined_name, virtual_output_combined_name,
};

/// Nodes are exactly two grid squares wide.
pub const GRID_STEP: f32 = 100.0;
pub const NODE_WIDTH: f32 = 2.0 * GRID_STEP;

pub const HEADER_H: f32 = 34.0;
pub const PORT_H: f32 = 16.0;
pub const PORT_PAD: f32 = 5.0;
pub const TITLE_FONT: f32 = 13.0;
pub const SUB_FONT: f32 = 9.5;
pub const PORT_FONT: f32 = 10.0;

pub struct GraphNode {
    pub id: u32,
    pub title: String,
    pub subtitle: Option<String>,
    pub category: PwNodeCategory,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub size: Vec2,
    pub component: usize,
}

impl GraphNode {
    pub fn rect(&self, center: Pos2) -> Rect {
        Rect::from_center_size(center, self.size)
    }

    pub fn input_pos(&self, center: Pos2, index: usize) -> Pos2 {
        pos2(self.rect(center).left(), center.y + self.port_offset(index))
    }

    pub fn output_pos(&self, center: Pos2, index: usize) -> Pos2 {
        pos2(
            self.rect(center).right(),
            center.y + self.port_offset(index),
        )
    }

    pub fn port_offset(&self, index: usize) -> f32 {
        HEADER_H + PORT_PAD + (index as f32 + 0.5) * PORT_H - self.size.y / 2.0
    }
}

pub struct GraphLink {
    pub from_node: usize,
    pub from_port: usize,
    pub to_node: usize,
    pub to_port: usize,
}

#[derive(Default)]
pub struct GraphModel {
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
    pub hidden_meters: usize,
    pub components: usize,
}

impl GraphModel {
    /// Cheap identity of the topology, to decide when to re-arrange.
    pub fn signature(&self) -> u64 {
        let mut hash: u64 = 1469598103934665603;
        let mut feed = |value: u64| {
            hash ^= value;
            hash = hash.wrapping_mul(1099511628211);
        };
        for node in &self.nodes {
            feed(node.id as u64);
            feed((node.inputs.len() * 64 + node.outputs.len()) as u64);
        }
        for link in &self.links {
            feed(((link.from_node * 4096 + link.to_node) as u64) << 20);
        }
        hash
    }
}

pub fn build(state: &PwState, config: &AppConfig, hide_meters: bool) -> GraphModel {
    let mut ports_by_node: HashMap<u32, Vec<(u32, &PwPort)>> = HashMap::new();
    for (object_id, object) in state {
        if let PwObject::Port(port) = object {
            ports_by_node
                .entry(port.node_id)
                .or_default()
                .push((*object_id, port));
        }
    }
    for ports in ports_by_node.values_mut() {
        ports.sort_by_key(|(object_id, port)| (port.port_id, *object_id));
    }

    let mut model = GraphModel::default();
    // Global port object id -> (node index, index within its direction).
    let mut port_index: HashMap<u32, (usize, PortDirection, usize)> = HashMap::new();

    let mut nodes = state
        .iter()
        .filter_map(|(id, object)| match object {
            PwObject::Node(node) => Some((*id, node)),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Stable order, so the layout does not shift when the map rehashes.
    nodes.sort_by_key(|(id, _)| *id);

    for (node_id, node) in nodes {
        if hide_meters && matches!(node.category, PwNodeCategory::PipemeeterMeter) {
            model.hidden_meters += 1;
            continue;
        }

        let ports = ports_by_node
            .get(&node_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if !is_audio_node(node, ports) {
            continue;
        }

        let node_index = model.nodes.len();
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();

        for (object_id, port) in ports {
            if matches!(port.media_type, PwMediaType::Video | PwMediaType::Midi) {
                continue;
            }
            let list = match port.direction {
                PortDirection::In => &mut inputs,
                PortDirection::Out => &mut outputs,
            };
            port_index.insert(*object_id, (node_index, port.direction, list.len()));
            list.push(port.name.clone());
        }

        let rows = inputs.len().max(outputs.len()) as f32;
        let height = (HEADER_H + 2.0 * PORT_PAD + rows * PORT_H).max(HEADER_H + 12.0);
        let (title, subtitle) = node_labels(node, config);

        model.nodes.push(GraphNode {
            id: node_id,
            title,
            subtitle,
            category: node.category,
            inputs,
            outputs,
            size: vec2(NODE_WIDTH, height),
            component: 0,
        });
    }

    for object in state.values() {
        let PwObject::Link(link) = object else {
            continue;
        };
        let Some(&(from_node, PortDirection::Out, from_port)) = port_index.get(&link.output_port)
        else {
            continue;
        };
        let Some(&(to_node, PortDirection::In, to_port)) = port_index.get(&link.input_port) else {
            continue;
        };
        model.links.push(GraphLink {
            from_node,
            from_port,
            to_node,
            to_port,
        });
    }

    model.components = assign_components(&mut model.nodes, &model.links);
    model
}

/// Union-find over the links, ignoring direction. Returns the component count.
fn assign_components(nodes: &mut [GraphNode], links: &[GraphLink]) -> usize {
    let mut parent = (0..nodes.len()).collect::<Vec<_>>();

    fn root(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }

    for link in links {
        let (a, b) = (
            root(&mut parent, link.from_node),
            root(&mut parent, link.to_node),
        );
        if a != b {
            parent[a] = b;
        }
    }

    // Renumber densely, by first appearance, so ids are stable per frame.
    let mut ids: HashMap<usize, usize> = HashMap::new();
    for index in 0..nodes.len() {
        let root = root(&mut parent, index);
        let next = ids.len();
        nodes[index].component = *ids.entry(root).or_insert(next);
    }
    ids.len()
}

/// The media class decides; nodes without one go by whether they carry audio.
fn is_audio_node(node: &PwNode, ports: &[(u32, &PwPort)]) -> bool {
    if let Some(media_class) = node.media_class.as_deref() {
        return media_class.contains("Audio");
    }
    ports
        .iter()
        .any(|(_, port)| matches!(port.media_type, PwMediaType::Audio))
}

fn node_labels(node: &PwNode, config: &AppConfig) -> (String, Option<String>) {
    if let Some(strip) = strip_name(config, &node.name) {
        return (strip.to_owned(), Some(node.name.clone()));
    }

    let title = node
        .description
        .as_deref()
        .or(node.nick.as_deref())
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(&node.name);

    let subtitle = node
        .media_name
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(&node.name);

    (
        title.to_owned(),
        (subtitle != title).then(|| subtitle.to_owned()),
    )
}

/// The name the user gave one of our own virtual devices.
fn strip_name<'a>(config: &'a AppConfig, node_name: &str) -> Option<&'a str> {
    let find = |strips: &'a [StripConfig], name: fn(usize) -> String| {
        (0..strips.len())
            .find(|&index| name(index) == node_name)
            .map(|index| strips[index].name.as_str())
    };

    find(&config.virtual_inputs, virtual_input_combined_name)
        .or_else(|| find(&config.virtual_outputs, virtual_output_combined_name))
}
