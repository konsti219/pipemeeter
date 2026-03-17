use crate::config::NodeMatchRequirement;
use crate::pipewire_backend::PwNodeCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Group {
    Physical,
    Virtual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct StripTarget {
    pub index: usize,
    pub category: PwNodeCategory,
}

impl StripTarget {
    pub(super) fn new(index: usize, category: PwNodeCategory) -> Self {
        Self { index, category }
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
