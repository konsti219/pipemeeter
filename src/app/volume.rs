pub(super) fn human_slider_to_pipewire_linear(human_slider: f32) -> f32 {
    let clamped = human_slider.clamp(0.0, 1.0);
    clamped * clamped * clamped
}
