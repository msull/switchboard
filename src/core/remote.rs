//! The remote port's state: the host's identity, the devices paired
//! with it, and the pairing window. Random material (the code, a
//! token) is made outside and arrives in the action; here it is only
//! compared and kept, so every transition is a plain function of its
//! inputs.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::SystemTime;

use uuid::Uuid;

use super::action::{
    AppAction, AppCore, Clock, Effect, Out, PAIRING_ATTEMPTS, PAIRING_FAILURES_MAX,
    PAIRING_SOURCES_MAX, PAIRING_WINDOW, PairingSource, PairingWindow,
};
use crate::ports::remote::{Identity, PairedDevice, Scope};

/// A device's last-seen time is saved when it moved by at least this
/// much, so a polling device does not rewrite `devices.json` on every
/// request.
const LAST_SEEN_SAVE_INTERVAL_SECS: u64 = 300;

impl AppCore {
    pub(super) fn remote_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::IdentityLoaded {
                identity,
                devices,
                rotated,
            } => {
                self.identity = Some(identity);
                self.devices = devices;
                self.identity_rotated_at = rotated.then_some(now.mono);
            }
            AppAction::SetServerName(name) => {
                let name = name.trim().to_owned();
                if !name.is_empty() {
                    self.update_identity(out, |i| i.server_name = name);
                }
            }
            AppAction::SetRemoteListen { lan, port } => {
                if port != 0 {
                    self.update_identity(out, |i| {
                        i.listen.lan = lan;
                        i.listen.port = port;
                    });
                }
            }
            AppAction::OpenPairing { code, scope } => {
                // Typed by hand on the device, so case never matters.
                self.pairing = Some(PairingWindow {
                    until: now.mono + PAIRING_WINDOW,
                    code: code.trim().to_ascii_uppercase(),
                    scope,
                    failures: 0,
                    sources: Vec::new(),
                });
            }
            AppAction::ClosePairing => self.pairing = None,
            AppAction::PairingFailed => {
                if let Some(p) = &mut self.pairing {
                    p.failures += 1;
                }
            }
            AppAction::PairingFailedFrom(ip) => {
                let Some(p) = &mut self.pairing else {
                    return;
                };
                let ip = ip.to_canonical();
                if p.is_locked(ip) {
                    return;
                }
                p.failures += 1;
                // Rotating source addresses gets three guesses per
                // address, so the window closes once the total reaches
                // what a full source list of lockouts would hold.
                if p.failures >= PAIRING_FAILURES_MAX {
                    self.pairing = None;
                    self.error("Pairing closed after too many failed attempts.".to_owned());
                } else if count_failure(&mut p.sources, ip) >= PAIRING_ATTEMPTS {
                    self.error(format!(
                        "Pairing locked out {ip} after three failed attempts."
                    ));
                }
            }
            AppAction::Paired(device) => {
                // The code is single-use: the window closes with the
                // first pairing made through it.
                self.pairing = None;
                self.devices.push(device);
                self.save_devices(out);
            }
            AppAction::DeviceSeen(id) => {
                let Some(device) = self.devices.iter_mut().find(|d| d.id == id) else {
                    return;
                };
                let stale = device.last_seen.is_none_or(|seen| {
                    now.wall
                        .duration_since(seen)
                        .is_ok_and(|d| d.as_secs() >= LAST_SEEN_SAVE_INTERVAL_SECS)
                });
                device.last_seen = Some(now.wall);
                if stale {
                    self.save_devices(out);
                }
            }
            AppAction::RevokeDevice(id) => {
                let before = self.devices.len();
                self.devices.retain(|d| d.id != id);
                if self.devices.len() != before {
                    self.save_devices(out);
                }
            }
            AppAction::SetDeviceScope { device, scope } => {
                if let Some(d) = self.devices.iter_mut().find(|d| d.id == device)
                    && d.scope != scope
                {
                    d.scope = scope;
                    self.save_devices(out);
                }
            }
            _ => unreachable!("dispatched by `dispatch` itself"),
        }
    }

    fn update_identity(&mut self, out: &mut Out, change: impl FnOnce(&mut Identity)) {
        let Some(identity) = &self.identity else {
            return;
        };
        let mut next = identity.clone();
        change(&mut next);
        if Some(&next) != self.identity.as_ref() {
            self.identity = Some(next.clone());
            if !self.read_only {
                out.push(Effect::SaveIdentity(next));
            }
        }
    }

    fn save_devices(&self, out: &mut Out) {
        if !self.read_only {
            out.push(Effect::SaveDevices(self.devices.clone()));
        }
    }

    /// The host's identity, once loaded.
    #[must_use]
    pub fn identity(&self) -> Option<&Identity> {
        self.identity.as_ref()
    }

    /// The key was remade at this launch: every earlier pairing is gone.
    #[must_use]
    pub fn identity_rotated(&self) -> bool {
        self.identity_rotated_at.is_some()
    }

    /// The devices paired with this host.
    #[must_use]
    pub fn devices(&self) -> &[PairedDevice] {
        &self.devices
    }

    /// The device holding the token with this hash.
    #[must_use]
    pub fn device_by_token(&self, token_sha256: &str) -> Option<&PairedDevice> {
        self.devices.iter().find(|d| d.token_sha256 == token_sha256)
    }

    /// The pairing window, while it is open at `now`. `Tick` clears a
    /// passed one; between ticks this checks the time itself.
    #[must_use]
    pub fn pairing_at(&self, now: Clock) -> Option<&PairingWindow> {
        self.pairing.as_ref().filter(|p| p.until > now.mono)
    }

    /// The pairing window as last set, passed or not; the dialog uses
    /// `pairing_at` and this is for tests.
    #[must_use]
    pub fn pairing(&self) -> Option<&PairingWindow> {
        self.pairing.as_ref()
    }

    /// A device record for a pairing that matched: the window's scope,
    /// the token's hash, and the time.
    #[must_use]
    pub fn new_device(&self, name: &str, token_sha256: String, at: SystemTime) -> PairedDevice {
        PairedDevice {
            id: Uuid::new_v4(),
            name: name.trim().chars().take(64).collect(),
            scope: self.pairing.as_ref().map_or(Scope::Read, |p| p.scope),
            token_sha256,
            created: at,
            last_seen: None,
        }
    }
}

/// Counts one failure from `ip` and returns that source's total. At
/// the cap the oldest unlocked source makes room; `PAIRING_FAILURES_MAX`
/// closes the window before every entry can be locked, so one is
/// always there, and a locked one is the fallback only to stay total.
fn count_failure(sources: &mut Vec<PairingSource>, ip: IpAddr) -> u32 {
    if let Some(s) = sources.iter_mut().find(|s| s.ip == ip) {
        s.failures += 1;
        return s.failures;
    }
    if sources.len() >= PAIRING_SOURCES_MAX {
        let oldest = sources
            .iter()
            .position(|s| s.failures < PAIRING_ATTEMPTS)
            .unwrap_or(0);
        sources.remove(oldest);
    }
    sources.push(PairingSource { ip, failures: 1 });
    1
}

/// The address a phone on the network reaches the port at: this Mac's
/// `local` address, only while the port listens on the network. `None`
/// means the invite names the loopback, which a phone cannot reach.
#[must_use]
pub fn invite_address(identity: &Identity, local: Option<IpAddr>, port: u16) -> Option<SocketAddr> {
    local
        .filter(|_| identity.listen.lan)
        .map(|ip| SocketAddr::new(ip, port))
}

/// The pairing invite the QR code and "Copy invite" carry: the page's
/// URL at `address`, with the code, the key to compare and the server
/// id in the fragment. A fragment is never sent in a request, so the
/// code reaches no server before the page has checked the key. The
/// address is the one `invite_address` picks; without one, this Mac's
/// loopback, which no phone can reach, so a caller showing the invite
/// must also say when `invite_address` found nothing (the pairing
/// dialog says the port answers this Mac only).
#[must_use]
pub fn invite_url(
    identity: &Identity,
    window: &PairingWindow,
    local: Option<IpAddr>,
    port: u16,
) -> String {
    let address = invite_address(identity, local, port)
        .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    format!(
        "https://{}/#code={}&key={}&id={}",
        address, window.code, identity.spki_sha256, identity.server_id
    )
}
