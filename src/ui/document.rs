//! Read-only preview of one file of a project: rendered Markdown, plain
//! text, or a note for what cannot be shown. Reloads when the file
//! changes on disk.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{Frame, RichText, Ui};
use egui_commonmark::CommonMarkViewer;

use super::{DrawCtx, GAP, PAD};
use crate::core::{AppAction, ProjectId};

/// Files above this are not read; the preview says so instead.
pub const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;

/// The loaded document.
#[derive(Debug, Clone)]
pub struct Preview {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Markdown(String),
    Text(String),
    /// Raw bytes of a PNG, JPEG, GIF, or WebP; egui decodes them.
    Image(Vec<u8>),
    Binary,
    TooLarge,
    Missing(String),
}

impl Preview {
    /// Read `path` for display. Never fails: problems become the body.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let meta = std::fs::metadata(path);
        let (modified, size) = meta
            .as_ref()
            .map(|m| (m.modified().ok(), m.len()))
            .unwrap_or_default();
        let body = match meta {
            Err(e) => Body::Missing(e.to_string()),
            Ok(m) if m.is_dir() => Body::Missing("this is a directory".into()),
            Ok(m) if m.len() > MAX_PREVIEW_BYTES => Body::TooLarge,
            Ok(_) => match std::fs::read(path) {
                Err(e) => Body::Missing(e.to_string()),
                Ok(bytes) => body_of(path, bytes),
            },
        };
        Self {
            path: path.to_path_buf(),
            modified,
            size,
            body,
        }
    }

    /// Whether the file on disk has changed since this was loaded.
    #[must_use]
    pub fn stale(&self) -> bool {
        let now = std::fs::metadata(&self.path)
            .map(|m| (m.modified().ok(), m.len()))
            .unwrap_or_default();
        now != (self.modified, self.size)
    }
}

fn body_of(path: &Path, bytes: Vec<u8>) -> Body {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if matches!(
        ext.as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    ) {
        return Body::Image(bytes);
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Body::Binary;
    };
    if text.bytes().take(8192).any(|b| b == 0) {
        return Body::Binary;
    }
    match ext.as_deref() {
        Some("md" | "markdown") => Body::Markdown(text),
        _ => Body::Text(text),
    }
}

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId, path: &Path) {
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    let fresh = cx
        .state
        .preview
        .as_ref()
        .is_some_and(|p| p.path == path && !p.stale());
    if !fresh {
        cx.state.preview = Some(Preview::load(path));
    }
    let Some(preview) = cx.state.preview.clone() else {
        return;
    };
    header(cx, ui, pid, &preview);
    egui::ScrollArea::vertical()
        .id_salt("document")
        .auto_shrink(false)
        .show(ui, |ui| {
            Frame::new()
                .fill(ui.visuals().panel_fill)
                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                .corner_radius(4)
                .inner_margin(PAD)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match &preview.body {
                        Body::Markdown(text) => {
                            CommonMarkViewer::new().show(ui, &mut cx.state.markdown, text);
                        }
                        Body::Text(text) => {
                            // egui's built-in highlighter knows Rust, C-likes,
                            // Python, and TOML; everything else is plain.
                            let lang = preview
                                .path
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("");
                            let theme =
                                egui_extras::syntax_highlighting::CodeTheme::from_style(ui.style());
                            egui_extras::syntax_highlighting::code_view_ui(ui, &theme, text, lang);
                        }
                        Body::Image(bytes) => {
                            let uri = format!("bytes://{}", preview.path.display());
                            ui.add(
                                egui::Image::from_bytes(uri, bytes.clone())
                                    .max_width(ui.available_width())
                                    .fit_to_original_size(1.0),
                            );
                        }
                        Body::Binary => weak(ui, "Binary file; open it in another app."),
                        Body::TooLarge => weak(ui, "Too large to preview; open it in another app."),
                        Body::Missing(why) => weak(ui, &format!("Cannot read this file: {why}")),
                    }
                });
        });
}

fn weak(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).weak());
}

fn header(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId, preview: &Preview) {
    let root = cx.core.workspace(pid).map(|w| w.project.root.clone());
    let rel = root
        .as_ref()
        .and_then(|r| preview.path.strip_prefix(r).ok())
        .map(Path::to_path_buf);
    let pinned = rel.as_ref().is_some_and(|rel| {
        cx.core
            .workspace(pid)
            .is_some_and(|w| w.project.pinned.contains(rel))
    });
    let name = preview
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(4)
        .inner_margin(PAD)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.heading(&name);
                let shown = rel.as_ref().unwrap_or(&preview.path);
                ui.label(RichText::new(shown.display().to_string()).weak());
                ui.label(RichText::new(size_text(preview.size)).weak().small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Back").clicked() {
                        cx.dispatch(AppAction::Back);
                    }
                    if let Some(rel) = &rel {
                        let label = if pinned { "Unpin" } else { "Pin" };
                        if ui.button(label).clicked() {
                            cx.dispatch(if pinned {
                                AppAction::UnpinDocument(pid, rel.clone())
                            } else {
                                AppAction::PinDocument(pid, rel.clone())
                            });
                        }
                    }
                    if ui.button("Copy path").clicked() {
                        ui.ctx().copy_text(preview.path.display().to_string());
                    }
                    if ui.button("Reveal").clicked() {
                        cx.dispatch(AppAction::RevealDocument(preview.path.clone()));
                    }
                    if ui.button("Open in editor").clicked() {
                        cx.dispatch(AppAction::OpenInEditor(preview.path.clone()));
                    }
                    if ui.button("Open").on_hover_text("Default app").clicked() {
                        cx.dispatch(AppAction::OpenDocument(preview.path.clone()));
                    }
                });
            });
        });
}

/// `1.2 KB`, `340 B`, `3.0 MB`.
#[must_use]
pub fn size_text(size: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let s = size as f64;
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", s / 1024.0)
    } else {
        format!("{:.1} MB", s / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_text_binary_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("a.md");
        std::fs::write(&md, "# hi").unwrap();
        assert_eq!(Preview::load(&md).body, Body::Markdown("# hi".into()));
        let txt = dir.path().join("a.rs");
        std::fs::write(&txt, "fn main() {}").unwrap();
        assert_eq!(Preview::load(&txt).body, Body::Text("fn main() {}".into()));
        let bin = dir.path().join("a.bin");
        std::fs::write(&bin, [0u8, 159, 146, 150]).unwrap();
        assert_eq!(Preview::load(&bin).body, Body::Binary);
        let png = dir.path().join("a.png");
        std::fs::write(&png, [137u8, 80, 78, 71]).unwrap();
        assert!(matches!(Preview::load(&png).body, Body::Image(_)));
        let p = Preview::load(&dir.path().join("nope"));
        assert!(matches!(p.body, Body::Missing(_)));
        let loaded = Preview::load(&md);
        assert!(!loaded.stale());
        std::fs::write(&md, "# hi there").unwrap();
        assert!(loaded.stale());
    }

    #[test]
    fn sizes() {
        assert_eq!(size_text(340), "340 B");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(3 * 1024 * 1024), "3.0 MB");
    }
}
