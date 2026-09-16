fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "promptbox embed spike",
        options,
        Box::new(|cc| Ok(Box::new(promptbox_embed_spike::Host::new(cc)))),
    )
}
