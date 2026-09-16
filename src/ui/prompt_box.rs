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
    /// Use the real clipboard and save dialog. Tests turn this off.
    pub native: bool,
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
    // A draft primed before the editor existed (a cloned session's
    // prompt) becomes the editor's first text.
    let primed = if boxes.editors.contains_key(&record.id) {
        None
    } else {
        cx.state.input_drafts.remove(&record.id)
    };
    let boxes = &mut cx.state.prompt_boxes;
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
        if native {
            editor.set_saver(Box::new(promptbox::adapters::saver::NativeSaver));
        }
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
    boxes.voice.set_captions_enabled(captions);
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
    if interrupt {
        cx.dispatch(AppAction::Interrupt(record.id));
    }
}

/// The rail's "Listening" line: which session the voice goes into, a
/// click to go there, and Stop.
pub fn rail_indicator(cx: &mut DrawCtx<'_>, ui: &mut Ui, compact: bool) {
    let Some(id) = cx.state.prompt_boxes.listening() else {
        return;
    };
    let name = cx
        .core
        .session(id)
        .map_or_else(|| "?".to_owned(), |s| s.name.clone());
    let p = theme::palette(ui);
    let green = egui::Color32::from_rgb(0x2e, 0xb8, 0x5c);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let label = if compact {
            "●".to_owned()
        } else {
            format!("● Listening · {name}")
        };
        if ui
            .add(
                egui::Button::new(RichText::new(label).color(green).text_style(theme::meta()))
                    .frame(false),
            )
            .on_hover_text(format!("Dictating into {name}; click to go there"))
            .clicked()
        {
            cx.dispatch(AppAction::ShowSession(id));
        }
        if !compact && theme::ghost_muted(ui, "Stop listening").clicked() {
            cx.state.prompt_boxes.stop();
        }
    });
    let _ = p;
    ui.add_space(6.0);
}

/// Panel height when it first opens; the user drags it afterwards.
#[must_use]
pub fn default_height() -> f32 {
    DEFAULT_HEIGHT
}
