//! Switchboard, as Dispatch sees it: one request in, one reply out, over
//! the control socket. The trait is what tests replace.

use std::io;
use std::path::{Path, PathBuf};

use switchboard_control::{Client, Reply, Request, SOCKET_FILE};

pub trait Port: Send {
    fn call(&mut self, request: &Request) -> io::Result<Reply>;
}

/// A connection to the socket, made on the first call and remade after
/// an error, so a Switchboard restart costs one failed request.
pub struct SocketPort {
    path: PathBuf,
    client: Option<Client>,
}

impl SocketPort {
    #[must_use]
    pub fn new(switchboard_data_dir: &Path) -> Self {
        Self {
            path: switchboard_data_dir.join(SOCKET_FILE),
            client: None,
        }
    }

    /// `SWITCHBOARD_DATA_DIR`, else Switchboard's default directory.
    pub fn from_env() -> anyhow::Result<Self> {
        if let Some(dir) = std::env::var_os("SWITCHBOARD_DATA_DIR") {
            return Ok(Self::new(Path::new(&dir)));
        }
        let dirs = directories::ProjectDirs::from("com", "sadburger", "Switchboard")
            .ok_or_else(|| anyhow::anyhow!("no home directory"))?;
        Ok(Self::new(dirs.data_dir()))
    }
}

impl Port for SocketPort {
    fn call(&mut self, request: &Request) -> io::Result<Reply> {
        if self.client.is_none() {
            self.client = Some(Client::connect(&self.path)?);
        }
        let result = self.client.as_mut().expect("connected above").call(request);
        if result.is_err() {
            self.client = None;
        }
        result
    }
}
