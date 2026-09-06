//! Desktop entry point: assembles the real adapters and starts the app.

use std::path::PathBuf;

use switchboard::SwitchboardApp;
use switchboard::adapters::agents::Agents;
use switchboard::adapters::ghostty::MacOpener;
use switchboard::adapters::hooks::{HookLog, WakeSocket, write_hook_settings};
use switchboard::adapters::store::JsonStore;
use switchboard::adapters::tmux::TmuxHost;
use switchboard::adapters::transcript::ClaudeTranscripts;
use switchboard::app::Services;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("switchboard=info"))
        .init();

    // `SWITCHBOARD_DATA_DIR` overrides the data directory for testing.
    let data_dir = std::env::var_os("SWITCHBOARD_DATA_DIR").map_or_else(
        || JsonStore::default_dir().expect("data directory"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&data_dir).expect("create data directory");

    let tmux_conf = data_dir.join("tmux.conf");
    TmuxHost::write_default_config(&tmux_conf).expect("write tmux config");
    let socket = std::env::var("SWITCHBOARD_TMUX_SOCKET")
        .unwrap_or_else(|_| TmuxHost::default_socket().to_owned());
    let host = TmuxHost::new(&socket, Some(tmux_conf));

    // The hook helper lives next to this binary.
    let hook_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("switchboard-hook")))
        .expect("hook helper path");
    if let Err(e) = write_hook_settings(&data_dir, &hook_bin) {
        log::error!("could not write hook settings: {e}");
    }

    // The running process sets its own Dock icon; without this macOS shows
    // a generic one while the app is open. Raw RGBA from scripts/icon.sh.
    let icon = egui::IconData {
        rgba: include_bytes!("../assets/icon-256.rgba").to_vec(),
        width: 256,
        height: 256,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([600.0, 400.0])
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "Switchboard",
        options,
        Box::new(move |cc| {
            // Image previews decode through egui's loaders.
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            let wake = match WakeSocket::bind_with(&data_dir, move || ctx.request_repaint()) {
                Ok(w) => Some(w),
                Err(e) => {
                    log::error!("wake socket: {e}");
                    None
                }
            };
            let services = Services {
                store: Box::new(JsonStore::new(data_dir.clone())),
                host: Box::new(host),
                events: Box::new(HookLog::new(data_dir.clone())),
                agents: Box::new(Agents::detect(data_dir.clone())),
                opener: Box::new(MacOpener::detect()),
                transcripts: Box::new(ClaudeTranscripts),
                wake,
            };
            let mut app = SwitchboardApp::with_services(services);
            app.start();
            // Dev aid: a script of actions to run at startup.
            if let Some(path) = std::env::var_os("SWITCHBOARD_SCRIPT") {
                match std::fs::read_to_string(&path) {
                    Ok(text) => switchboard::script::run(&mut app, &text),
                    Err(e) => log::error!("script {}: {e}", path.to_string_lossy()),
                }
            }
            Ok(Box::new(app))
        }),
    )
}
