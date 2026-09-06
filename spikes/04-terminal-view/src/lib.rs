//! Spike: an egui 0.36 window with an embedded terminal (egui_term on top of
//! alacritty_terminal) running the user's shell. A status bar shows frames
//! per second, frame time and the pty id so rendering cost can be observed.

use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use egui_term::{BackendSettings, PtyEvent, TerminalBackend, TerminalView};

pub struct SpikeApp {
    backend: TerminalBackend,
    events: Receiver<(u64, PtyEvent)>,
    frames: VecDeque<Instant>,
    started: Instant,
    /// Wall time of the previous frame's ui pass, for the status bar.
    last_frame_ms: f32,
    pub exited: bool,
}

impl SpikeApp {
    /// `command`: run this through `$SHELL -lc` instead of an interactive shell.
    pub fn new(ctx: egui::Context, command: Option<String>) -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let args = match command {
            Some(cmd) if !cmd.is_empty() => vec!["-lc".into(), cmd],
            _ => vec!["-l".into()],
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let backend = TerminalBackend::new(
            0,
            ctx,
            tx,
            BackendSettings {
                shell,
                args,
                working_directory: std::env::current_dir().ok(),
            },
        )
        .expect("spawn shell in pty");
        Self {
            backend,
            events: rx,
            frames: VecDeque::new(),
            started: Instant::now(),
            last_frame_ms: 0.0,
            exited: false,
        }
    }

    /// The visible grid as text, one line per row, trailing blanks trimmed.
    /// Used by the headless keyboard test to read what the shell echoed.
    pub fn screen_text(&self) -> String {
        let content = self.backend.last_content();
        let mut rows: Vec<String> = Vec::new();
        let mut line = String::new();
        for indexed in content.grid.display_iter() {
            if indexed.point.column.0 == 0 && !(rows.is_empty() && line.is_empty()) {
                rows.push(line.trim_end().to_string());
                line.clear();
            }
            line.push(indexed.c);
        }
        rows.push(line.trim_end().to_string());
        rows.join("\n")
    }
}

impl eframe::App for SpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Ok((_, PtyEvent::Exit)) = self.events.try_recv() {
            self.exited = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let frame_start = Instant::now();
        self.frames.push_back(frame_start);
        while self
            .frames
            .front()
            .is_some_and(|t| frame_start.duration_since(*t).as_secs_f32() > 1.0)
        {
            self.frames.pop_front();
        }

        egui::Panel::top("status").show(ui, |ui| {
            ui.monospace(format!(
                "fps(1s window) {:>3}   frame {:>5.2}ms   uptime {:>5.0}s   pty {}",
                self.frames.len(),
                self.last_frame_ms,
                self.started.elapsed().as_secs_f32(),
                self.backend.pty_id(),
            ));
        });

        let terminal = TerminalView::new(ui, &mut self.backend)
            .set_focus(true)
            .set_size(ui.available_size());
        ui.add(terminal);
        self.last_frame_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
    }
}
