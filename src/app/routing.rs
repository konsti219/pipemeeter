use crate::pipewire_backend::{
    PwObject, virtual_input_combined_name, virtual_output_combined_name,
};

use super::PipeMeeterApp;

impl PipeMeeterApp {
    pub(super) fn virtual_input_node_id(&self, index: usize) -> Option<u32> {
        self.managed_node_id(&virtual_input_combined_name(index))
    }

    pub(super) fn virtual_output_node_id(&self, index: usize) -> Option<u32> {
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
}
