use crate::pipewire_backend::{PwObject, VIRTUAL_DEVICE_PREFIX};

use super::PipeMeeterApp;

fn virtual_input_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vin-{}", index + 1)
}

fn virtual_output_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vout-{}", index + 1)
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
