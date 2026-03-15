pub(super) fn pipewire_stereo_to_human_slider(stereo_volume: [f32; 2]) -> f32 {
    let average_linear = ((stereo_volume[0] + stereo_volume[1]) * 0.5).max(0.0);
    pipewire_linear_to_human_slider(average_linear)
}

pub(super) fn human_slider_to_pipewire_linear(human_slider: f32) -> f32 {
    let clamped = human_slider.clamp(0.0, 1.0);
    clamped * clamped * clamped
}

fn pipewire_linear_to_human_slider(linear_volume: f32) -> f32 {
    // PipeWire exposes linear gain. A cubic-root curve gives a more perceptual UI scale.
    let human = linear_volume.cbrt();
    (human * 100.0).round() / 100.0
}
