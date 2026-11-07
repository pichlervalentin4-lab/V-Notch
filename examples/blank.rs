//! A blank transparent eframe window of the notch's size: the baseline for memory measurements.

use eframe::egui;

fn main() -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([820.0, 500.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top();
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    eframe::run_ui_native("blank", options, |ui, _frame| {
        ui.label("blank");
    })
}
