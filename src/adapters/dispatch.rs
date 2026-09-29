//! The Dispatch port over `<Dispatch data dir>/dispatch.sock`, connected
//! on the first call and reconnected after an error, so a runner
//! restart costs one failed request. The data directory is
//! `DISPATCH_DATA_DIR` or Dispatch's default, the same rule the
//! `dispatch` binary uses; the command is the `dispatch` beside this
//! executable (the bundle carries both), else the bare name.

use std::io;
use std::path::PathBuf;

use dispatch_control::{Body, Client, Reply, Request, SOCKET_FILE};

use crate::ports::dispatch::DispatchPort;

pub struct DispatchSocket {
    data_dir: PathBuf,
    command: PathBuf,
    client: Option<Client>,
    next_op: u64,
}

impl DispatchSocket {
    #[must_use]
    pub fn new(data_dir: PathBuf, command: PathBuf) -> Self {
        Self {
            data_dir,
            command,
            client: None,
            next_op: 1,
        }
    }

    /// The directory the `dispatch` binary would use, and the binary
    /// next to this one.
    #[must_use]
    pub fn detect() -> Self {
        let data_dir = std::env::var_os("DISPATCH_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                directories::ProjectDirs::from("", "", "Dispatch")
                    .map(|d| d.data_dir().to_path_buf())
            })
            .unwrap_or_else(|| PathBuf::from("."));
        let command = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("dispatch")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("dispatch"));
        Self::new(data_dir, command)
    }

    fn socket(&self) -> PathBuf {
        self.data_dir.join(SOCKET_FILE)
    }
}

impl DispatchPort for DispatchSocket {
    fn command(&self) -> PathBuf {
        self.command.clone()
    }

    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn call(&mut self, body: &Body) -> io::Result<Reply> {
        if self.client.is_none() {
            self.client = Some(Client::connect(&self.socket())?);
        }
        let op = format!("sb-{}-{}", std::process::id(), self.next_op);
        self.next_op += 1;
        let request = Request::new(op, body.clone());
        let result = self
            .client
            .as_mut()
            .expect("connected above")
            .call(&request);
        if result.is_err() {
            self.client = None;
        }
        result
    }
}

/// A socket path a test can bind: under the temp directory, short.
#[cfg(test)]
#[must_use]
pub fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sbd-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    use super::*;

    #[test]
    fn no_runner_is_an_error_and_a_runner_is_asked_with_fresh_ops() {
        let dir = test_dir("port");
        let mut port = DispatchSocket::new(dir.clone(), PathBuf::from("/x/dispatch"));
        assert_eq!(port.command(), PathBuf::from("/x/dispatch"));
        assert!(port.call(&Body::Status).is_err(), "no socket yet");
        let listener = UnixListener::bind(dir.join(SOCKET_FILE)).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut ops = Vec::new();
            for _ in 0..2 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req = Request::parse(line.trim_end()).unwrap();
                ops.push(req.op);
                let mut w = stream.try_clone().unwrap();
                w.write_all(Reply::Artifact { text: "t".into() }.to_line().as_bytes())
                    .unwrap();
            }
            ops
        });
        assert!(matches!(
            port.call(&Body::Status),
            Ok(Reply::Artifact { .. })
        ));
        assert!(matches!(
            port.call(&Body::Status),
            Ok(Reply::Artifact { .. })
        ));
        let ops = server.join().unwrap();
        assert_ne!(ops[0], ops[1], "every request is its own operation");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
