pub fn slider_to_pipewire_linear(slider: f32) -> f32 {
    let clamped = slider.max(0.0);
    clamped * clamped * clamped
}

pub fn pipewire_linear_to_slider(linear: f32) -> f32 {
    linear.max(0.0).cbrt()
}
