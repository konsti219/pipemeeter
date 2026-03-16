use crate::config::NodeMatchRequirement;
use crate::pipewire_backend::PwNodeCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Group {
    Physical,
    Virtual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum StripTarget {
    Input { group: Group, index: usize },
    Output { group: Group, index: usize },
}

impl StripTarget {
    pub(super) fn node_filter(self) -> PwNodeCategory {
        match self {
            StripTarget::Input { group, .. } => match group {
                Group::Physical => PwNodeCategory::InputDevice,
                Group::Virtual => PwNodeCategory::PlaybackStream,
            },
            StripTarget::Output { group, .. } => match group {
                Group::Physical => PwNodeCategory::OutputDevice,
                Group::Virtual => PwNodeCategory::RecordingStream,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct EditDialogState {
    pub target: StripTarget,
    pub draft_strip_name: String,
    pub draft_represented_node_requirements: Vec<NodeMatchRequirement>,
    pub selected_requirement_index: usize,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedNodeEntry {
    pub id: u32,
    pub display_text: String,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedNodeInfo {
    pub nodes: Vec<ResolvedNodeEntry>,
}
