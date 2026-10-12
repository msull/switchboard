//! The remote port's durable files: the host's identity (`identity.json`)
//! and the devices paired with it (`devices.json`). Both are small files
//! beside `views.json`, written through the store's atomic path. The
//! certificate's private key is not here: it is a Keychain item under
//! [`Identity::key_account`].

use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The file names inside the data directory.
pub const IDENTITY_FILE: &str = "identity.json";
pub const DEVICES_FILE: &str = "devices.json";

/// The version of `identity.json` this build writes.
pub const IDENTITY_SCHEMA_VERSION: u32 = 1;

/// The port the host listens on unless the identity says otherwise.
pub const DEFAULT_REMOTE_PORT: u16 = 43187;

/// Who this installation is to a remote client, and how it listens.
/// Made once on first start and kept for the life of the data
/// directory; a second data directory has its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub schema_version: u32,
    /// Stable, random; never derived from the hostname or an address.
    pub server_id: Uuid,
    /// The display name a client shows for this host.
    pub server_name: String,
    /// The self-signed certificate, DER, base64. Public.
    pub certificate: String,
    /// SHA-256 over the certificate's `SubjectPublicKeyInfo`, hex: what
    /// a client pins, so a certificate reissued over the same key still
    /// verifies.
    pub spki_sha256: String,
    pub created: SystemTime,
    /// How the port binds.
    #[serde(default)]
    pub listen: Listen,
}

impl Identity {
    /// The Keychain account the certificate's private key is filed
    /// under: one per installation, so two data directories under one
    /// macOS account never overwrite each other's keys.
    #[must_use]
    pub fn key_account(&self) -> String {
        format!("remote-key:{}", self.server_id)
    }
}

/// Where the remote port binds. Loopback only until the owner allows
/// devices on the local network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listen {
    /// Bind every interface rather than loopback alone.
    pub lan: bool,
    pub port: u16,
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            lan: false,
            port: DEFAULT_REMOTE_PORT,
        }
    }
}

/// What a paired device may do. Set in the host's dialog, never over
/// the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Every view and content read.
    Read,
    /// Reads, and the commands that put text into a session.
    Control,
}

impl Scope {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Control => "control",
        }
    }
}

/// One device the owner paired. The bearer token itself is never
/// stored: its SHA-256 is what a request's token is checked against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: Uuid,
    /// What the device called itself when it paired.
    pub name: String,
    pub scope: Scope,
    /// SHA-256 of the bearer token, hex.
    pub token_sha256: String,
    pub created: SystemTime,
    /// When the device last made an authenticated request.
    #[serde(default)]
    pub last_seen: Option<SystemTime>,
}

/// The two files on disk. A missing file reads as absent or empty; a
/// malformed one is an error the app reports rather than overwrites.
pub trait RemoteStore: Send {
    fn load_identity(&self) -> Result<Option<Identity>, String>;
    fn save_identity(&self, identity: &Identity) -> Result<(), String>;
    fn load_devices(&self) -> Result<Vec<PairedDevice>, String>;
    fn save_devices(&self, devices: &[PairedDevice]) -> Result<(), String>;
}
