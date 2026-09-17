//! The Prompt Box editor as an agent session's message box. One
//! [`Editor`] per session keeps its own draft and undo history in memory;
//! one [`Voice`] runtime, bound to at most one session, keeps listening
//! while other views are on screen. Send hands the prompt to the
//! session's pane; nothing here touches the clipboard or any file of the
//! standalone Prompt Box app.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui::{RichText, Ui};
use promptbox::adapters::clipboard::{FakeClipboard, SystemClipboard};
use promptbox::adapters::persistence::MemoryStore;
use promptbox::ports::saver::FileSaver;
use promptbox::ports::sink::PromptSink;
use promptbox::{Editor, Voice};

use super::cards::is_running;
use super::{DrawCtx, UiState, theme};
use crate::core::{AppAction, RecordId, SessionRecord, VOICE_KEY_ACCOUNT};

/// Prompts sent from editors, waiting for the frame to end so they can
/// be dispatched with the app borrowed mutably.
pub type Outbox = Arc<Mutex<Vec<(RecordId, String)>>>;

/// Where a session's editor sends: the pane, if it is running. The flag
/// is refreshed every frame before the editor draws, so a Send into a
/// dead session fails at once and the prompt stays.
struct PaneSink {
    id: RecordId,
    running: Arc<AtomicBool>,
    outbox: Outbox,
}

impl PromptSink for PaneSink {
    fn deliver(&mut self, text: &str) -> Result<(), String> {
        if !self.running.load(Ordering::Relaxed) {
            return Err("the session is not running".into());
        }
        self.outbox
            .lock()
            .expect("outbox mutex")
            .push((self.id, text.to_owned()));
        Ok(())
    }
}

/// Paths the editors saved to, waiting for the frame to refresh the file
/// side of the project they landed in.
pub type SavedPaths = Arc<Mutex<Vec<std::path::PathBuf>>>;

/// The saver the editors get: the native dialog (or the fake in tests)
/// with every saved path noted, so the file tree can pick it up.
struct NotingSaver {
    inner: Box<dyn FileSaver>,
    saved: SavedPaths,
}

impl FileSaver for NotingSaver {
    fn save(
        &mut self,
        seed: &std::path::Path,
        text: &str,
    ) -> Result<Option<std::path::PathBuf>, String> {
        let result = self.inner.save(seed, text)?;
        if let Some(path) = &result {
            self.saved.lock().expect("saved paths").push(path.clone());
        }
        Ok(result)
    }
}

/// The Keychain is read once per run, on the first editor.
#[derive(Debug, Clone, Default)]
enum KeyState {
    #[default]
    Unread,
    Read(Option<String>),
}

/// Everything the embedded editors share.
pub struct PromptBoxes {
    pub editors: std::collections::HashMap<RecordId, Editor>,
    /// The per-session "pane is running" flags the sinks read.
    running: std::collections::HashMap<RecordId, Arc<AtomicBool>>,
    pub voice: Voice,
    /// The session the voice runtime dictates into.
    pub bound: Option<RecordId>,
    pub outbox: Outbox,
    /// The `OpenAI` key as last read from the Keychain.
    key: KeyState,
    /// Use the real clipboard and save dialog. Tests turn this off and
    /// get a fake saver that saves to `test_save_path` without asking.
    pub native: bool,
    pub test_save_path: Option<std::path::PathBuf>,
    /// Files saved from the editors since the last frame.
    pub saved: SavedPaths,
    /// The captions setting last pushed into the runtime, so the CC
    /// button's own change is not overwritten on the next frame.
    applied_captions: Option<bool>,
}

impl Default for PromptBoxes {
    fn default() -> Self {
        Self {
            editors: std::collections::HashMap::new(),
            running: std::collections::HashMap::new(),
            voice: Voice::default(),
            bound: None,
            outbox: Outbox::default(),
            key: KeyState::Unread,
            native: true,
            test_save_path: None,
            saved: SavedPaths::default(),
            applied_captions: None,
        }
    }
}

impl PromptBoxes {
    /// The session being listened to, while the runtime is live.
    #[must_use]
    pub fn listening(&self) -> Option<RecordId> {
        self.bound
            .filter(|_| self.voice.is_live() || self.voice.is_demo_running())
    }

    /// Stops listening (or the demo), wherever it was bound.
    pub fn stop(&mut self) {
        let mut actions = self.voice.stop();
        actions.extend(self.voice.stop_demo());
        if let Some(editor) = self.bound.and_then(|id| self.editors.get_mut(&id)) {
            editor.apply(actions);
        }
    }

    /// Starts (or moves) listening into `id`'s editor. Another session's
    /// utterance in progress is finished into its own editor first.
    fn listen_into(&mut self, id: RecordId) {
        if self.bound != Some(id) {
            self.stop();
            self.bound = Some(id);
        }
        let Some(editor) = self.editors.get_mut(&id) else {
            return;
        };
        let actions = self.voice.start(editor.vocabulary_hint());
        editor.apply(actions);
    }

    /// The key is stored once per app run; a change pushes into every editor.
    pub fn set_key(&mut self, key: Option<String>) {
        for editor in self.editors.values_mut() {
            editor.set_api_key(key.clone());
        }
        self.key = KeyState::Read(key);
    }
}

/// The per-frame work, wherever the app is: the runtime's audio into the
/// bound editor, every editor's workers, editors of gone sessions
/// dropped. Returns how soon a repaint is wanted.
pub fn pump(cx: &mut DrawCtx<'_>) -> Option<std::time::Duration> {
    let core = cx.core;
    // A file saved from an editor shows up in its project's tree at once.
    let saved: Vec<std::path::PathBuf> =
        std::mem::take(&mut *cx.state.prompt_boxes.saved.lock().expect("saved paths"));
    for path in saved {
        for workspace in core.workspaces() {
            if path.starts_with(&workspace.project.root)
                && let Some(files) = cx.state.files.get_mut(&workspace.project.id)
            {
                files.refresh();
            }
        }
    }
    let boxes = &mut cx.state.prompt_boxes;
    boxes.editors.retain(|id, _| core.session(*id).is_some());
    boxes.running.retain(|id, _| core.session(*id).is_some());
    if let Some(bound) = boxes.bound
        && !boxes.editors.contains_key(&bound)
    {
        boxes.voice.stop();
        boxes.bound = None;
    }
    let pumped = boxes.voice.pump();
    let mut repaint = pumped.repaint;
    if let Some(editor) = boxes.bound.and_then(|id| boxes.editors.get_mut(&id)) {
        editor.apply(pumped.actions);
    }
    let mut stop = false;
    for (id, editor) in &mut boxes.editors {
        let delay = editor.pump();
        repaint = match (repaint, delay) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if editor.take_stop_request() && boxes.bound == Some(*id) {
            stop = true;
        }
    }
    if stop {
        boxes.stop();
    }
    repaint
}

/// The caption and preview overlays of the bound editor.
pub fn overlays(state: &mut UiState, ctx: &egui::Context) {
    let boxes = &mut state.prompt_boxes;
    let Some(editor) = boxes.bound.and_then(|id| boxes.editors.get_mut(&id)) else {
        return;
    };
    promptbox::caption::draw(&mut boxes.voice, editor.core(), ctx);
    promptbox::preview::draw(editor, ctx);
}

/// The session's editor, made on first sight with the session's own
/// sink, save directory, and the shared key and settings.
fn editor_for<'a>(cx: &'a mut DrawCtx<'_>, record: &SessionRecord) -> &'a mut Editor {
    let settings = cx.core.settings().voice.clone();
    let root = cx
        .core
        .workspace(record.project)
        .map(|w| w.project.root.clone());
    let key_source = cx.services;
    let boxes = &mut cx.state.prompt_boxes;
    if matches!(boxes.key, KeyState::Unread) {
        let key = match key_source.secrets.get(VOICE_KEY_ACCOUNT) {
            Ok(key) => key,
            Err(e) => {
                log::warn!("could not read the OpenAI key: {e}");
                None
            }
        };
        boxes.key = KeyState::Read(key);
    }
    let running = boxes
        .running
        .entry(record.id)
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone();
    let native = boxes.native;
    let outbox = boxes.outbox.clone();
    let saved = boxes.saved.clone();
    let test_save_path = boxes.test_save_path.clone();
    // A primed draft (a cloned session's prompt, or the one a discard
    // cut back to) becomes the editor's text, first or replacing.
    let primed = cx.state.primed.remove(&record.id);
    let boxes = &mut cx.state.prompt_boxes;
    if let Some(text) = &primed
        && let Some(editor) = boxes.editors.get_mut(&record.id)
    {
        editor.set_text(text);
    }
    let key = match &boxes.key {
        KeyState::Read(key) => key.clone(),
        KeyState::Unread => None,
    };
    let editor = boxes.editors.entry(record.id).or_insert_with(|| {
        let clipboard: Box<dyn promptbox::ports::clipboard::Clipboard> = if native {
            Box::new(SystemClipboard::default())
        } else {
            Box::new(FakeClipboard::default())
        };
        let mut editor = Editor::with_services(clipboard, Box::new(MemoryStore::default()));
        editor.set_id(egui::Id::new(("prompt-box", record.id)));
        editor.set_embedded(true);
        editor.set_sink(Box::new(PaneSink {
            id: record.id,
            running,
            outbox,
        }));
        let inner: Box<dyn FileSaver> = if native {
            Box::new(promptbox::adapters::saver::NativeSaver)
        } else {
            Box::new(promptbox::adapters::saver::FakeSaver {
                choose: test_save_path,
                ..Default::default()
            })
        };
        editor.set_saver(Box::new(NotingSaver { inner, saved }));
        editor.set_save_dir(root);
        editor.set_api_key(key);
        if let Some(text) = primed {
            editor.set_text(&text);
        }
        editor
    });
    // Settings changed in the menu reach every editor as it is drawn.
    let current = editor.settings();
    if current.trigger != settings.trigger
        || current.openai_model != settings.openai_model
        || current.captions != settings.captions
    {
        editor.settings_draft.trigger = settings.trigger;
        editor.settings_draft.openai_model = settings.openai_model;
        editor.settings_draft.captions = settings.captions;
        editor.save_settings_draft();
    }
    editor
}

/// Height the editor panel opens at.
const DEFAULT_HEIGHT: f32 = 300.0;

/// The message panel of an agent session: a host row (Stop) above the
/// Prompt Box editor. Files dropped on it land in the prompt as paths.
pub fn panel(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let running = is_running(cx.core, record.id);
    let captions = cx.core.settings().voice.captions;
    let dropped: Vec<std::path::PathBuf> = ui.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .collect()
    });
    // One slim row of the host's own controls; everything else is
    // Prompt Box's, so the editor gets the height.
    let mut interrupt = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled_ui(running, |ui| theme::ghost_muted(ui, "Stop agent"))
                .inner
                .on_hover_text("Send Escape to the agent (Cmd+.)")
                .clicked()
            {
                interrupt = true;
            }
        });
    });
    let editor = editor_for(cx, record);
    for path in dropped {
        let mut text = editor.core().doc().rendered();
        super::session::append_path(&mut text, &path);
        editor.set_text(&text);
    }
    let boxes = &mut cx.state.prompt_boxes;
    if let Some(flag) = boxes.running.get(&record.id) {
        flag.store(running, Ordering::Relaxed);
    }
    // The stored setting reaches the runtime when it changes; the CC
    // button changes the runtime and is written back below.
    if boxes.applied_captions != Some(captions) {
        boxes.voice.set_captions_enabled(captions);
        boxes.applied_captions = Some(captions);
    }
    let bound = boxes.bound == Some(record.id);
    let mut listen = None;
    if let Some(editor) = boxes.editors.get_mut(&record.id) {
        let mut frame = promptbox::ui::Frame {
            editor,
            voice: &mut boxes.voice,
            window: None,
            bound,
            listen: None,
        };
        promptbox::ui::draw(&mut frame, ui);
        listen = frame.listen;
    }
    match listen {
        Some(true) => boxes.listen_into(record.id),
        Some(false) if bound => boxes.stop(),
        _ => {}
    }
    let toggled = boxes.voice.captions_enabled();
    if toggled != captions {
        boxes.applied_captions = Some(toggled);
        let voice = cx.core.settings().voice.clone();
        cx.dispatch(AppAction::SetVoiceSettings(crate::core::VoiceSettings {
            captions: toggled,
            ..voice
        }));
    }
    if interrupt {
        cx.dispatch(AppAction::Interrupt(record.id));
    }
}

/// The rail's listening row, always the same one row so nothing else
/// moves: a green dot and "Listening · <session>" with a Stop beside it while
/// the runtime is live (the name goes to the session), a muted "Not
/// listening" that does nothing otherwise. The name truncates rather
/// than widening the rail.
pub fn rail_indicator(cx: &mut DrawCtx<'_>, ui: &mut Ui, compact: bool) {
    let p = theme::palette(ui);
    let green = egui::Color32::from_rgb(0x2e, 0xb8, 0x5c);
    let listening = cx.state.prompt_boxes.listening().map(|id| {
        let name = cx
            .core
            .session(id)
            .map_or_else(|| "?".to_owned(), |s| s.name.clone());
        (id, name)
    });
    let height = ui.spacing().interact_size.y;
    ui.horizontal(|ui| {
        ui.set_height(height);
        ui.spacing_mut().item_spacing.x = 6.0;
        // A painted dot, as on the cards: the text font has no circle glyph.
        let fill = if listening.is_some() { green } else { p.n400 };
        let (dot, _) = ui.allocate_exact_size(egui::Vec2::splat(8.0), egui::Sense::hover());
        ui.painter().circle_filled(dot.center(), 4.0, fill);
        if compact {
            return;
        }
        let Some((id, name)) = listening else {
            // Left-aligned in the row's full width, as the live label is.
            let label = egui::Label::new(
                RichText::new("Not listening")
                    .text_style(theme::meta())
                    .color(p.n500),
            )
            .truncate();
            let size = egui::vec2(ui.available_width(), height);
            ui.allocate_ui_with_layout(
                size,
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.add(label),
            );
            return;
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // A label like the idle one, so the row is one height whether
            // or not it is listening.
            let stop = egui::Label::new(
                RichText::new("Stop listening")
                    .text_style(theme::meta())
                    .color(p.n600),
            )
            .sense(egui::Sense::click());
            if ui
                .add(stop)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                cx.state.prompt_boxes.stop();
            }
            let live = egui::Label::new(
                RichText::new(format!("Listening · {name}"))
                    .color(green)
                    .text_style(theme::meta()),
            )
            .truncate()
            .sense(egui::Sense::click());
            let size = egui::vec2(ui.available_width(), height);
            let clicked = ui
                .allocate_ui_with_layout(
                    size,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add(live)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(format!("Dictating into {name}; click to go there"))
                            .clicked()
                    },
                )
                .inner;
            if clicked {
                cx.dispatch(AppAction::ShowSession(id));
            }
        });
    });
}

/// Start listening into `record`'s editor from anywhere (a card), making
/// the editor first if the session has not been opened yet; a second
/// call while listening there stops it.
pub fn toggle_listening(cx: &mut DrawCtx<'_>, record: &SessionRecord) {
    if cx.state.prompt_boxes.listening() == Some(record.id) {
        cx.state.prompt_boxes.stop();
        return;
    }
    editor_for(cx, record);
    cx.state.prompt_boxes.listen_into(record.id);
}

/// A small painted microphone (the text fonts have none): green while
/// listening into the session, muted otherwise. Named for tests and
/// screen readers as "Listen here" or "Listening here".
pub fn mic_button(ui: &mut Ui, listening: bool) -> egui::Response {
    let p = theme::palette(ui);
    let size = egui::vec2(14.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let label = if listening {
        "Listening here"
    } else {
        "Listen here"
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let color = if listening {
        egui::Color32::from_rgb(0x2e, 0xb8, 0x5c)
    } else if response.hovered() {
        p.n700
    } else {
        p.n500
    };
    let painter = ui.painter();
    let c = rect.center();
    // Capsule head, a cradle under it, and the stand.
    let head =
        egui::Rect::from_center_size(egui::pos2(c.x, rect.top() + 5.0), egui::vec2(5.0, 9.0));
    painter.rect_filled(head, 2.5, color);
    let cradle =
        egui::Rect::from_center_size(egui::pos2(c.x, rect.top() + 6.5), egui::vec2(10.0, 9.5));
    painter.rect_stroke(
        cradle,
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 5,
            se: 5,
        },
        egui::Stroke::new(1.2, color),
        egui::StrokeKind::Middle,
    );
    painter.line_segment(
        [
            egui::pos2(c.x, cradle.bottom()),
            egui::pos2(c.x, rect.bottom() - 1.0),
        ],
        egui::Stroke::new(1.2, color),
    );
    painter.line_segment(
        [
            egui::pos2(c.x - 3.0, rect.bottom() - 1.0),
            egui::pos2(c.x + 3.0, rect.bottom() - 1.0),
        ],
        egui::Stroke::new(1.2, color),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Panel height when it first opens; the user drags it afterwards.
#[must_use]
pub fn default_height() -> f32 {
    DEFAULT_HEIGHT
}
