//! Desktop entry point: assembles the real adapters and starts the app.

use std::path::PathBuf;

use switchboard::SwitchboardApp;
use switchboard::adapters::agents::Agents;
use switchboard::adapters::ghostty::MacOpener;
use switchboard::adapters::hooks::{HookLog, WakeSocket, write_hook_settings};
use switchboard::adapters::project_config::FileConfigReader;
use switchboard::adapters::store::JsonStore;
use switchboard::adapters::tmux::TmuxHost;
use switchboard::adapters::transcript::ClaudeTranscripts;
use switchboard::app::Services;

/// Screen points are whole numbers far below what `f32` counts exactly.
#[allow(clippy::cast_precision_loss)]
fn points(v: i32) -> f32 {
    v as f32
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("switchboard=info"))
        .init();
    // A GUI app starts with a soft limit of 256 open files on macOS;
    // embedded terminals, tmux clients, and log pipes each take a few,
    // so the limit is raised to what the system allows.
    match rlimit::increase_nofile_limit(8192) {
        Ok(limit) => log::info!("open file limit {limit}"),
        Err(e) => log::warn!("could not raise the open file limit: {e}"),
    }

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
    if let Err(e) = host.apply_options() {
        log::warn!("tmux options: {e}");
    }

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
    // Where the window was last seen, if that display is attached: a
    // window zoomed to its screen comes back as that frame. Otherwise
    // zoomed to the main screen, with a size to fall back to when
    // un-zoomed. Native points: zoom is applied from the first frame on.
    let store = JsonStore::new(data_dir.clone());
    let viewport = egui::ViewportBuilder::default().with_min_inner_size([600.0, 400.0]);
    let loaded = store.load_settings().main_window;
    let screens: Vec<String> = promptbox::adapters::screens::screens()
        .into_iter()
        .map(|s| s.name)
        .collect();
    let saved = loaded
        .clone()
        .filter(|f| switchboard::ui::zoom::monitor_attached(&f.monitor));
    let decision =
        format!("main window: saved {loaded:?}; displays {screens:?}; restoring {saved:?}");
    log::info!("{decision}");
    // A Dock launch has no stderr, so the decision is also left in the
    // data directory for the last launch.
    if let Err(e) = std::fs::write(data_dir.join("launch.log"), format!("{decision}\n")) {
        log::warn!("could not write launch.log: {e}");
    }
    let viewport = match saved {
        Some(f) => viewport
            .with_position([points(f.x), points(f.y)])
            .with_inner_size([points(f.w), points(f.h)]),
        None => viewport
            .with_inner_size([1100.0, 720.0])
            .with_maximized(true),
    };
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_icon(icon)
            // Transparency is decided once, here, for every viewport: the
            // caption overlay of the embedded Prompt Box needs it. The
            // window's own panels paint opaque backgrounds regardless.
            .with_transparent(true),
        ..Default::default()
    };

    eframe::run_native(
        "Switchboard",
        options,
        Box::new(move |cc| {
            // Image previews decode through egui's loaders.
            egui_extras::install_image_loaders(&cc.egui_ctx);
            // Fonts and styles apply from the next frame on, so they go
            // in before the first one is drawn.
            switchboard::ui::theme::install(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            let wake = match WakeSocket::bind_with(&data_dir, move || ctx.request_repaint()) {
                Ok(w) => Some(w),
                Err(e) => {
                    log::error!("wake socket: {e}");
                    None
                }
            };
            let services = Services {
                store: Box::new(store),
                host: Box::new(host),
                events: Box::new(HookLog::new(data_dir.clone())),
                agents: Box::new(Agents::detect(data_dir.clone())),
                opener: Box::new(MacOpener::detect()),
                transcripts: Box::new(ClaudeTranscripts),
                secrets: secret_store(),
                project_config: Box::new(FileConfigReader::new()),
                round_files: Box::new(switchboard::adapters::round_files::DiskRoundFiles),
                artifacts: Box::new(switchboard::adapters::artifacts::DiskArtifacts),
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

/// The Keychain on macOS. Elsewhere the app only has to build (CI runs
/// on Linux), so secrets live in memory for the process.
fn secret_store() -> Box<dyn switchboard::ports::secrets::SecretStore> {
    #[cfg(target_os = "macos")]
    {
        Box::new(switchboard::adapters::keychain::KeychainStore::login())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(switchboard::adapters::fakes::FakeSecrets::default())
    }
}
