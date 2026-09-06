//! Dock badge: the number of sessions waiting on you, on the app's Dock
//! icon, so the to-do list of agents is visible without the window.
//!
//! macOS only; elsewhere the badge is a no-op.

/// Shows `count` on the Dock icon, or clears the badge for zero. Must be
/// called from the main thread (the eframe frame loop is fine).
pub fn set_waiting_badge(count: usize) {
    #[cfg(target_os = "macos")]
    {
        let label = (count > 0).then(|| count.to_string());
        macos::set_badge(label.as_deref());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    pub fn set_badge(label: Option<&str>) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("dock badge requested off the main thread; ignoring");
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let label = label.map(NSString::from_str);
        app.dockTile().setBadgeLabel(label.as_deref());
    }
}
