//! Read-only preview of one file of a project: rendered Markdown, plain
//! text, or a note for what cannot be shown. Reloads when the file
//! changes on disk. The full-screen view lives here; the file side's
//! bottom pane draws the same body through [`body`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use egui::{Frame, RichText, Ui};

use super::{DrawCtx, GAP, UiState, theme};
use crate::core::{AppAction, PinTarget, ProjectId};

/// Files above this are not read; the preview says so instead.
pub const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;

/// How often the file on disk is compared with the loaded copy. A stat
/// per frame is wasted work; a change shows up within this delay.
const STALE_CHECK_EVERY: Duration = Duration::from_millis(500);

/// The loaded document.
#[derive(Debug, Clone)]
pub struct Preview {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
    pub body: Body,
    /// When the disk was last compared with this copy.
    checked: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Markdown(String),
    Text(String),
    /// Raw bytes of a PNG, JPEG, GIF, or WebP; egui decodes them.
    Image(Vec<u8>),
    /// A PDF: its first page is rasterized on demand (`pdf_page`).
    Pdf,
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
            checked: Instant::now(),
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
    if ext.as_deref() == Some("pdf") {
        return Body::Pdf;
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

/// Load `path` into the state's preview slot unless the copy there is
/// still current.
pub fn ensure_loaded(state: &mut UiState, path: &Path) {
    ensure_in(&mut state.preview, path);
}

/// Load `path` into `slot` unless the copy there is still current.
pub fn ensure_in(slot: &mut Option<Preview>, path: &Path) {
    let fresh = slot.as_mut().is_some_and(|p| {
        if p.path != path {
            return false;
        }
        if p.checked.elapsed() < STALE_CHECK_EVERY {
            return true;
        }
        p.checked = Instant::now();
        !p.stale()
    });
    if !fresh {
        *slot = Some(Preview::load(path));
    }
}

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId, path: &Path) {
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    ensure_loaded(cx.state, path);
    // The header needs only the path and size, so the body (up to
    // `MAX_PREVIEW_BYTES`) stays in the state instead of being cloned
    // every frame.
    let Some(size) = cx.state.preview.as_ref().map(|p| p.size) else {
        return;
    };
    header(cx, ui, pid, path, size);
    // Prose wraps at the visible width, but a table or a wide image
    // cannot, so the area also scrolls sideways for those. The width
    // is taken before the scroll area, which offers unbounded room.
    let width = ui.available_width();
    egui::ScrollArea::both()
        .id_salt("document")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.set_max_width(width.min(MAX_READING_WIDTH));
            Frame::new()
                .inner_margin(egui::Margin::symmetric(0, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    body(cx.state, ui);
                });
        });
}

/// Prose stops here, however wide the window.
const MAX_READING_WIDTH: f32 = 860.0;

/// Give Markdown the design's colors: cyan links, a dark code block on
/// both themes, inline code on a neutral tint. The viewer reads these
/// from the egui style, and the change is scoped to `ui` (a style set
/// on a `Ui` lives only as long as that `Ui`), so nothing else on
/// screen shifts.
pub fn markdown_style(ui: &mut Ui) {
    let p = theme::palette(ui);
    let visuals = &mut ui.style_mut().visuals;
    visuals.hyperlink_color = p.accent_text;
    visuals.extreme_bg_color = p.code_fill;
    visuals.code_bg_color = p.n300;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    // The code block is dark on both themes, so its highlighting has to
    // be the dark theme's: the highlighter reads this flag, and nothing
    // else drawn inside the scoped `ui` does.
    visuals.dark_mode = true;
}

/// Draw the loaded preview's contents: Markdown, highlighted text, an
/// image, or the note for what cannot be shown.
pub fn body(state: &mut UiState, ui: &mut Ui) {
    // Destructuring borrows two fields of `state` at once, which a method
    // call on `state` could not: the preview is read while the markdown
    // layout cache is written.
    let UiState {
        preview, markdown, ..
    } = state;
    let Some(preview) = preview else {
        return;
    };
    draw_body(preview, markdown, ui);
}

/// A file drawn wherever a state is at hand: loaded through the preview
/// cache, with a PDF's first page rasterized into `renders`.
pub fn show_file(state: &mut UiState, ui: &mut Ui, path: &Path, renders: &Path) {
    let slot = state.previews.entry(path.to_path_buf()).or_insert(None);
    ensure_in(slot, path);
    let is_pdf = slot.as_ref().is_some_and(|p| p.body == Body::Pdf);
    if is_pdf {
        pdf_page(state, ui, path, renders);
        return;
    }
    let UiState {
        previews, markdown, ..
    } = state;
    if let Some(preview) = previews.get(path).and_then(Option::as_ref) {
        draw_body(preview, markdown, ui);
    }
}

/// A PDF page being rasterized, or done.
#[derive(Debug)]
pub enum PdfRender {
    Pending(std::sync::mpsc::Receiver<Result<Vec<u8>, String>>),
    Ready(Vec<u8>),
    Failed(String),
}

/// Pixels across the rendered page.
const PDF_RENDER_SIZE: u32 = 1400;

/// The first page of the PDF at `path` as an image, rasterized once on
/// a thread by the system's Quick Look (`qlmanage`) into `renders` and
/// kept in memory after. Until then, a spinner.
pub fn pdf_page(state: &mut UiState, ui: &mut Ui, path: &Path, renders: &Path) {
    let entry = state
        .pdf_renders
        .entry(path.to_path_buf())
        .or_insert_with(|| PdfRender::Pending(rasterize(path, renders)));
    if let PdfRender::Pending(rx) = entry
        && let Ok(result) = rx.try_recv()
    {
        *entry = match result {
            Ok(bytes) => PdfRender::Ready(bytes),
            Err(e) => PdfRender::Failed(e),
        };
    }
    match entry {
        PdfRender::Pending(_) => {
            ui.spinner();
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        PdfRender::Ready(bytes) => {
            let uri = format!("bytes://pdf-{}", path.display());
            ui.add(
                egui::Image::from_bytes(uri, bytes.clone())
                    .max_width(ui.available_width())
                    .fit_to_original_size(1.0),
            );
        }
        PdfRender::Failed(e) => weak(ui, &format!("Cannot render this PDF: {e}")),
    }
}

/// Start the rasterization; the receiver gets the PNG bytes.
fn rasterize(path: &Path, renders: &Path) -> std::sync::mpsc::Receiver<Result<Vec<u8>, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (path, renders) = (path.to_path_buf(), renders.to_path_buf());
    std::thread::spawn(move || {
        let _ = tx.send(rasterize_now(&path, &renders));
    });
    rx
}

fn rasterize_now(path: &Path, renders: &Path) -> Result<Vec<u8>, String> {
    std::fs::create_dir_all(renders).map_err(|e| format!("{}: {e}", renders.display()))?;
    let status = std::process::Command::new("qlmanage")
        .arg("-t")
        .arg("-s")
        .arg(PDF_RENDER_SIZE.to_string())
        .arg("-o")
        .arg(renders)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("qlmanage: {e}"))?;
    if !status.success() {
        return Err(format!("qlmanage exited with {status}"));
    }
    let name = path
        .file_name()
        .ok_or_else(|| "no file name".to_owned())?
        .to_string_lossy()
        .to_string();
    let png = renders.join(format!("{name}.png"));
    let bytes = std::fs::read(&png).map_err(|e| format!("{}: {e}", png.display()))?;
    let _ = std::fs::remove_file(&png);
    Ok(bytes)
}

/// Draw one loaded preview's contents.
pub fn draw_body(preview: &Preview, markdown: &mut egui_commonmark::CommonMarkCache, ui: &mut Ui) {
    match &preview.body {
        Body::Markdown(text) => {
            markdown_style(ui);
            super::markdown::show(ui, markdown, text);
        }
        Body::Text(text) => {
            // egui's built-in highlighter knows Rust, C-likes, Python,
            // and TOML; everything else is plain.
            let lang = preview
                .path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let theme = egui_extras::syntax_highlighting::CodeTheme::from_style(ui.style());
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
        Body::Pdf => weak(ui, "PDF; its page shows where a card can render it."),
        Body::Binary => weak(ui, "Binary file; open it in another app."),
        Body::TooLarge => weak(ui, "Too large to preview; open it in another app."),
        Body::Missing(why) => weak(ui, &format!("Cannot read this file: {why}")),
    }
}

fn weak(ui: &mut Ui, text: &str) {
    ui.label(theme::meta_text(ui, text));
}

/// Below this width the actions go on their own row under the title.
const TIGHT_HEADER: f32 = 720.0;

fn header(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId, path: &Path, size: u64) {
    let project = cx.core.workspace(pid).map(|w| &w.project);
    let rel = project.and_then(|p| path.strip_prefix(&p.root).ok());
    let pinned = match (project, rel) {
        (Some(p), Some(rel)) => p.pinned.iter().any(|d| d == rel),
        _ => false,
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let title = |ui: &mut Ui| {
        let p = theme::palette(ui);
        ui.add(egui::Label::new(RichText::new(&name).text_style(theme::h1())).truncate());
        ui.label(RichText::new(size_text(size)).small().color(p.n600));
    };
    // The buttons claim the right end first and the title is cut to
    // what is left, so a long file name never runs under them. When
    // there is no room for both, the buttons get their own row.
    if ui.available_width() < TIGHT_HEADER {
        ui.horizontal(title);
        ui.horizontal_wrapped(|ui| header_actions(cx, ui, pid, path, rel, pinned, false));
    } else {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                header_actions(cx, ui, pid, path, rel, pinned, true);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), title);
            });
        });
    }
    let shown = rel.unwrap_or(path);
    ui.add(egui::Label::new(theme::mono_text(ui, shown.display().to_string())).truncate());
    ui.add_space(8.0);
}

/// Open, Open in editor, Reveal, Copy path, Pin, Back. `reversed`
/// draws them last to first, for a right-to-left row.
fn header_actions(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    pid: ProjectId,
    path: &Path,
    rel: Option<&Path>,
    pinned: bool,
    reversed: bool,
) {
    #[derive(Clone, Copy)]
    enum Item {
        Open,
        Editor,
        Reveal,
        CopyPath,
        Pin,
        WorkingSet,
        Back,
    }
    let mut items = vec![
        Item::Open,
        Item::Editor,
        Item::Reveal,
        Item::CopyPath,
        Item::Pin,
        Item::WorkingSet,
        Item::Back,
    ];
    if reversed {
        items.reverse();
    }
    ui.spacing_mut().item_spacing.x = 2.0;
    for item in items {
        match item {
            Item::Back => {
                if theme::ghost_muted(ui, "Back").clicked() {
                    cx.dispatch(AppAction::Back);
                }
            }
            Item::WorkingSet => {
                let Some(rel) = rel else { continue };
                let target = PinTarget::File(pid, rel.to_path_buf());
                let response = theme::ghost_muted(ui, "Working sets")
                    .on_hover_text("Which working sets show this file");
                egui::Popup::menu(&response).show(|ui| {
                    if super::working_set::set_menu(cx, ui, &target) {
                        ui.close();
                    }
                });
            }
            Item::Pin => {
                let Some(rel) = rel else { continue };
                let label = if pinned { "Unpin" } else { "Pin" };
                if theme::ghost(ui, label).clicked() {
                    cx.dispatch(if pinned {
                        AppAction::UnpinDocument(pid, rel.to_path_buf())
                    } else {
                        AppAction::PinDocument(pid, rel.to_path_buf())
                    });
                }
            }
            Item::CopyPath => {
                if theme::ghost(ui, "Copy path").clicked() {
                    ui.ctx().copy_text(path.display().to_string());
                }
            }
            Item::Reveal => {
                if theme::ghost(ui, "Reveal").clicked() {
                    cx.dispatch(AppAction::RevealDocument(path.to_path_buf()));
                }
            }
            Item::Editor => {
                if theme::ghost(ui, "Open in editor").clicked() {
                    cx.dispatch(AppAction::OpenInEditor(path.to_path_buf()));
                }
            }
            Item::Open => {
                if theme::ghost(ui, "Open")
                    .on_hover_text("Default app")
                    .clicked()
                {
                    cx.dispatch(AppAction::OpenDocument(path.to_path_buf()));
                }
            }
        }
    }
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
