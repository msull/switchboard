use terminal_view_spike::SpikeApp;

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("terminal-view spike")
            .with_inner_size([1000.0, 640.0]),
        ..Default::default()
    };
    // Dev aid: SPIKE_CMD="top" runs that command instead of an interactive
    // prompt, so TUIs can be screenshotted unattended.
    let command = std::env::var("SPIKE_CMD").ok();
    eframe::run_native(
        "terminal-view-spike",
        options,
        Box::new(move |cc| Ok(Box::new(SpikeApp::new(cc.egui_ctx.clone(), command)))),
    )
}
