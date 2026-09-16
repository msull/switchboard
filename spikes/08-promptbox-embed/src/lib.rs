//! Spike 8: can Prompt Box draw inside a Switchboard panel and hand its
//! Send straight to the host instead of the clipboard?
//!
//! The host is a stand-in for Switchboard's session view: a header, a
//! fake conversation in the middle, and Prompt Box in a bottom panel.

use std::sync::{Arc, Mutex};

use promptbox::PromptBoxApp;
use promptbox::adapters::persistence::MemoryStore;
use promptbox::ports::clipboard::Clipboard;

/// Sends land here instead of the system clipboard: Switchboard would
/// turn each one into `SendInput` for the session's pane.
#[derive(Debug, Default, Clone)]
pub struct PaneSink(pub Arc<Mutex<Vec<String>>>);

impl Clipboard for PaneSink {
    fn write_text(&mut self, text: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(text.to_owned());
        Ok(())
    }
}

pub struct Host {
    pub prompt: PromptBoxApp,
    pub sink: PaneSink,
    /// Lines "sent to the agent", shown above as the conversation.
    pub sent: Vec<String>,
}

impl Host {
    #[must_use]
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        promptbox::ui::install_symbol_font(&cc.egui_ctx);
        Self::headless()
    }

    /// Without a creation context (tests); the symbol font is optional.
    #[must_use]
    pub fn headless() -> Self {
        let sink = PaneSink::default();
        let prompt = PromptBoxApp::with_services(
            Box::new(sink.clone()),
            Box::new(MemoryStore::default()),
        );
        Self {
            prompt,
            sink,
            sent: Vec::new(),
        }
    }
}

/// Height of the embedded prompt box, as Switchboard's message panel.
pub const PROMPT_HEIGHT: f32 = 320.0;

impl eframe::App for Host {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Prompt Box's per-frame work: workers, recognizer, microphone.
        if let Some(delay) = self.prompt.pump() {
            ui.ctx().request_repaint_after(delay);
        }
        self.sent.extend(self.sink.0.lock().unwrap().drain(..));

        egui::Panel::top("sb-header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("claude-agent");
                ui.label("· switchboard · running");
            });
        });
        egui::Panel::bottom("sb-message")
            .exact_size(PROMPT_HEIGHT)
            .resizable(true)
            .show(ui, |ui| {
                promptbox::ui::draw(&mut self.prompt, ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(egui::RichText::new("You · 10:12").small().weak());
            ui.label("reply with the single word pong");
            ui.label(egui::RichText::new("Agent · 10:12").small().weak());
            ui.label("pong");
            for line in &self.sent {
                ui.label(egui::RichText::new("You · sent from Prompt Box").small().weak());
                ui.label(line);
            }
        });
    }
}
