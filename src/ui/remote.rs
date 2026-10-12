//! The Remote devices dialog: who this host is to a device, where it
//! listens, the pairing window with its QR code, and the devices paired
//! so far with their scope and a Revoke. Nothing here decides anything:
//! every change is an action, and the host's state is read back from
//! the core each frame.

use egui::{Context, RichText, Ui};

use super::{DrawCtx, GAP, cards::since_text, theme};
use crate::adapters::remote;
use crate::core::{AppAction, AppCore};
use crate::ports::remote::Scope;

/// What the dialog edits between frames: the name and port fields, the
/// scope the next pairing grants, and the QR image for the open code.
pub struct RemoteDraft {
    pub name: String,
    pub port: String,
    pub scope: Scope,
    /// The QR texture for the code it was made for.
    qr: Option<(String, egui::TextureHandle)>,
}

impl RemoteDraft {
    #[must_use]
    pub fn open(core: &AppCore) -> Self {
        let identity = core.identity();
        Self {
            name: identity.map(|i| i.server_name.clone()).unwrap_or_default(),
            port: identity
                .map(|i| i.listen.port.to_string())
                .unwrap_or_default(),
            scope: Scope::Read,
            qr: None,
        }
    }
}

/// The dialog's width: the QR and the device rows fit it.
const DIALOG_WIDTH: f32 = 620.0;
/// Pixels per QR module; the quiet zone is the standard four.
const QR_SCALE: usize = 4;
const QR_QUIET: usize = 4;

pub fn show(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(mut draft) = cx.state.remote_dialog.take() else {
        return;
    };
    let mut keep = true;
    egui::Window::new("")
        .id(egui::Id::new("remote-devices"))
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_width(DIALOG_WIDTH)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let screen = ctx.content_rect().width();
            ui.set_max_width((screen - 80.0).clamp(320.0, DIALOG_WIDTH));
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            ui.label(RichText::new("Remote devices").text_style(theme::brand()));
            ui.add_space(8.0);
            host(cx, ui, &mut draft);
            ui.add_space(8.0);
            pairing(cx, ui, &mut draft);
            ui.add_space(8.0);
            devices(cx, ui);
            ui.add_space(8.0);
            let p = theme::palette(ui);
            ui.label(
                RichText::new(
                    "What a device reads is redacted: the values of a project's secret variables and of its granted sets are replaced by their names. A secret pasted into a prompt, or printed by a tool, that Switchboard never held is not found.",
                )
                .small()
                .color(p.n600),
            );
            ui.horizontal(|ui| {
                if theme::ghost_muted(ui, "Close").clicked() {
                    keep = false;
                }
            });
        });
    if keep {
        cx.state.remote_dialog = Some(draft);
    }
}

/// This host: its name, id and key, and where it listens.
fn host(cx: &mut DrawCtx<'_>, ui: &mut Ui, draft: &mut RemoteDraft) {
    theme::section(ui, "This Mac");
    let Some(identity) = cx.core.identity().cloned() else {
        ui.label("The host has no identity yet; see the log.");
        return;
    };
    ui.horizontal(|ui| {
        let id = ui.label("Name").id;
        let field = ui
            .add(egui::TextEdit::singleline(&mut draft.name).desired_width(240.0))
            .labelled_by(id);
        if field.lost_focus() && draft.name.trim() != identity.server_name {
            cx.dispatch(AppAction::SetServerName(draft.name.clone()));
        }
    });
    ui.label(theme::meta_text(ui, format!("Id {}", identity.server_id)));
    ui.label(theme::meta_text(
        ui,
        format!("Key {}", short_hash(&identity.spki_sha256)),
    ));
    ui.horizontal(|ui| {
        let mut lan = identity.listen.lan;
        if ui
            .checkbox(&mut lan, "Allow devices on the local network")
            .on_hover_text("Off, the port answers this Mac alone; on, it listens on every interface and only paired devices get past /info")
            .changed()
        {
            cx.dispatch(AppAction::SetRemoteListen {
                lan,
                port: identity.listen.port,
            });
        }
        let id = ui.label("Port").id;
        let field = ui
            .add(egui::TextEdit::singleline(&mut draft.port).desired_width(64.0))
            .labelled_by(id);
        if field.lost_focus() {
            match draft.port.trim().parse::<u16>() {
                Ok(port) if port != 0 && port != identity.listen.port => {
                    cx.dispatch(AppAction::SetRemoteListen {
                        lan: identity.listen.lan,
                        port,
                    });
                }
                Ok(_) => {}
                Err(_) => draft.port = identity.listen.port.to_string(),
            }
        }
    });
}

/// The pairing window: open it with a scope, or show the code it holds.
fn pairing(cx: &mut DrawCtx<'_>, ui: &mut Ui, draft: &mut RemoteDraft) {
    theme::section(ui, "Pairing");
    let p = theme::palette(ui);
    if cx.core.identity_rotated() {
        ui.label(
            RichText::new(
                "The host's key was remade at this launch, so every earlier pairing is invalid. Pair each device again.",
            )
            .color(p.accent_2_text),
        );
    }
    let Some(identity) = cx.core.identity().cloned() else {
        return;
    };
    if let Some(window) = cx.core.pairing().cloned() {
        let address = remote::local_address()
            .map(|ip| format!("{ip}:{}", identity.listen.port))
            .unwrap_or_default();
        let payload = serde_json::json!({
            "server_id": identity.server_id,
            "spki_sha256": identity.spki_sha256,
            "code": window.code,
            "addresses": if address.is_empty() { Vec::new() } else { vec![address.clone()] },
        })
        .to_string();
        if draft
            .qr
            .as_ref()
            .is_none_or(|(code, _)| *code != window.code)
        {
            draft.qr = qr_texture(ui.ctx(), &payload).map(|t| (window.code.clone(), t));
        }
        ui.horizontal(|ui| {
            if let Some((_, texture)) = &draft.qr {
                ui.image(egui::load::SizedTexture::from_handle(texture));
            }
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(spaced(&window.code))
                        .text_style(theme::h1())
                        .family(egui::FontFamily::Monospace),
                );
                ui.label(theme::meta_text(
                    ui,
                    format!("Key {}", short_hash(&identity.spki_sha256)),
                ));
                if !address.is_empty() {
                    ui.label(theme::meta_text(ui, format!("Address {address}")));
                }
                ui.label(theme::meta_text(
                    ui,
                    format!(
                        "Scan, or type the key and the code. Grants {} scope. Open for five minutes or three wrong codes.",
                        window.scope.word()
                    ),
                ));
                if theme::secondary(ui, "Stop pairing").clicked() {
                    cx.dispatch(AppAction::ClosePairing);
                }
            });
        });
    } else {
        ui.horizontal(|ui| {
            ui.radio_value(&mut draft.scope, Scope::Read, "Read-only")
                .on_hover_text("Every view and the screen; nothing is sent into a session");
            ui.radio_value(&mut draft.scope, Scope::Control, "Control")
                .on_hover_text("Reads, and later the commands that type into a session");
            if theme::primary(ui, "Allow pairing for 5 minutes").clicked() {
                cx.dispatch(AppAction::OpenPairing {
                    code: remote::pairing_code(),
                    scope: draft.scope,
                });
            }
        });
    }
}

/// The devices paired so far, each with its scope and a Revoke.
fn devices(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    theme::section(ui, "Paired devices");
    let devices = cx.core.devices().to_vec();
    if devices.is_empty() {
        ui.label(theme::meta_text(ui, "No device is paired."));
        return;
    }
    egui::Grid::new("remote-devices")
        .num_columns(5)
        .spacing([GAP, GAP])
        .show(ui, |ui| {
            for device in devices {
                ui.label(&device.name);
                let mut scope = device.scope;
                egui::ComboBox::from_id_salt(("remote-scope", device.id))
                    .selected_text(scope.word())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut scope, Scope::Read, "read");
                        ui.selectable_value(&mut scope, Scope::Control, "control");
                    });
                if scope != device.scope {
                    cx.dispatch(AppAction::SetDeviceScope {
                        device: device.id,
                        scope,
                    });
                }
                ui.label(theme::meta_text(
                    ui,
                    format!("paired {}", since_text(device.created)),
                ));
                ui.label(theme::meta_text(
                    ui,
                    match device.last_seen {
                        Some(seen) => format!("seen {}", since_text(seen)),
                        None => "never seen".into(),
                    },
                ));
                if theme::ghost_muted(ui, "Revoke")
                    .on_hover_text("The device's next request fails; pair it again to restore it")
                    .clicked()
                {
                    cx.dispatch(AppAction::RevokeDevice(device.id));
                }
                ui.end_row();
            }
        });
}

/// The QR code for `payload` as a texture: black modules on white with
/// the quiet zone, scaled so a phone reads it from the screen.
fn qr_texture(ctx: &Context, payload: &str) -> Option<egui::TextureHandle> {
    let code = qrcode::QrCode::new(payload.as_bytes()).ok()?;
    let width = code.width();
    let modules = code.to_colors();
    let side = (width + 2 * QR_QUIET) * QR_SCALE;
    let mut pixels = vec![egui::Color32::WHITE; side * side];
    for (i, module) in modules.iter().enumerate() {
        if *module != qrcode::Color::Dark {
            continue;
        }
        let (x, y) = (i % width + QR_QUIET, i / width + QR_QUIET);
        for dy in 0..QR_SCALE {
            for dx in 0..QR_SCALE {
                pixels[(y * QR_SCALE + dy) * side + x * QR_SCALE + dx] = egui::Color32::BLACK;
            }
        }
    }
    let image = egui::ColorImage::new([side, side], pixels);
    Some(ctx.load_texture("remote-pairing-qr", image, egui::TextureOptions::NEAREST))
}

/// The first sixteen hex digits of a key hash in groups of four: what
/// a person compares against the client's screen.
fn short_hash(hex: &str) -> String {
    hex.chars()
        .take(16)
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// "ABCD EFGH": the code in two halves, as it is read aloud.
fn spaced(code: &str) -> String {
    let (a, b) = code.split_at(code.len() / 2);
    format!("{a} {b}")
}
