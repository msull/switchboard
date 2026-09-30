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
        let dirs = directories::ProjectDirs::from("", "", "Switchboard")
            .ok_or_else(|| anyhow::anyhow!("no home directory"))?;
        Ok(Self::new(dirs.data_dir()))
    }
}

impl Port for SocketPort {
    fn call(&mut self, request: &Request) -> io::Result<Reply> {
        let kept = self.client.is_some();
        let result = self.once(request);
        // A connection kept from before a Switchboard restart fails on
        // its first use, though nothing was received. One more try over
        // a fresh connection is safe even for a command: Switchboard
        // answers a repeated op from its operations log.
        match &result {
            Err(e) if kept && stale(e) => self.once(request),
            _ => result,
        }
    }
}

impl SocketPort {
    fn once(&mut self, request: &Request) -> io::Result<Reply> {
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

/// Whether an error says the other end went away, rather than that it
/// answered badly or is still working.
fn stale(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    use switchboard_control::Body;

    use super::*;

    /// A connection kept across a restart: the first call after it
    /// fails at the socket, and the port answers from a new one.
    #[test]
    fn a_stale_connection_is_remade_and_the_request_sent_again() {
        let dir = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(dir.path().join(SOCKET_FILE)).unwrap();
        let server = std::thread::spawn(move || {
            // The first connection answers one request, then closes
            // as a quitting Switchboard would.
            let (first, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(first.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut w = first;
            writeln!(w, r#"{{"reply":"spaces","spaces":[]}}"#).unwrap();
            drop(w);
            drop(reader);
            // The second connection is the relaunched Switchboard.
            let (second, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(second.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut w = second;
            writeln!(w, r#"{{"reply":"spaces","spaces":[]}}"#).unwrap();
            line
        });
        let mut port = SocketPort::new(dir.path());
        let first = Request::new("q-1".to_owned(), Body::Spaces);
        assert!(matches!(port.call(&first), Ok(Reply::Spaces { .. })));
        let second = Request::new("q-2".to_owned(), Body::Spaces);
        assert!(
            matches!(port.call(&second), Ok(Reply::Spaces { .. })),
            "answered over a fresh connection"
        );
        let seen = server.join().unwrap();
        assert!(seen.contains("q-2"), "the same request went again: {seen}");
    }
}
