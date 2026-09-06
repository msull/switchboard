//! Desktop entry point.

use switchboard::SwitchboardApp;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("switchboard=info"))
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 320.0])
            .with_min_inner_size([280.0, 200.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Switchboard",
        options,
        Box::new(|cc| Ok(Box::new(SwitchboardApp::new(cc)))),
    )
}
