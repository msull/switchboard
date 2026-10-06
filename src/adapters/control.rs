//! The control port's adapters: the operations log on disk, and the
//! socket that carries requests from another process.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use switchboard_control::{NO_ANSWER, Reply, Request, SOCKET_FILE};

use crate::ports::control::{OpLine, Operations};

/// The log's file name inside the data directory.
pub const OPERATIONS_FILE: &str = "operations.log";

/// `<data dir>/operations.log`: one JSON line per entry, appended with
/// `O_APPEND` and synced before `append` returns, owner-readable only.
/// Existing lines are read once at open and kept in memory for `find`.
pub struct OperationsLog {
    path: PathBuf,
    lines: Vec<OpLine>,
}

impl OperationsLog {
    /// Open (creating if absent) the log in `data_dir`.
    pub fn open(data_dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(OPERATIONS_FILE);
        let mut lines = Vec::new();
        match File::open(&path) {
            Ok(file) => {
                for line in BufReader::new(file).lines() {
                    let line = line?;
                    match serde_json::from_str::<OpLine>(&line) {
                        Ok(entry) => lines.push(entry),
                        // A torn last line from a crash mid-write is the
                        // one expected malformation; anything else is
                        // logged and skipped the same way.
                        Err(e) => log::warn!("{}: skipping a line: {e}", path.display()),
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(Self { path, lines })
    }

    /// Every line, oldest first.
    #[cfg(test)]
    #[must_use]
    pub fn lines(&self) -> &[OpLine] {
        &self.lines
    }
}

impl Operations for OperationsLog {
    fn append(&mut self, line: &OpLine) -> std::io::Result<()> {
        let mut text = serde_json::to_string(line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        text.push('\n');
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        self.lines.push(line.clone());
        Ok(())
    }

    fn find(&self, op: &str) -> Vec<OpLine> {
        self.lines
            .iter()
            .filter(|l| l.op() == op)
            .cloned()
            .collect()
    }
}

/// One request off the socket, with the channel its reply goes back on.
/// The connection thread waits on that channel and writes the line.
pub struct Incoming {
    pub request: Request,
    pub reply: mpsc::Sender<Reply>,
}

/// How long a connection waits for the app to answer before giving up on
/// the request. The app answers on its next frame, so this only matters
/// if the frame loop has stalled.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

/// A Unix socket path may be at most 104 bytes on macOS (108 on Linux);
/// binding a longer one fails obscurely, so it is refused with a reason.
const MAX_SOCKET_PATH: usize = 100;

/// `<data dir>/control.sock`, owner-only: one thread accepts, one thread
/// per connection reads lines and hands each request to the app over a
/// channel, then writes the reply the app sends back. Bound only by an
/// instance that holds the store lock, so a read-only second instance
/// never steals a live socket.
pub struct ControlSocket {
    path: PathBuf,
    requests: mpsc::Receiver<Incoming>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ControlSocket {
    /// Bind in `data_dir`. `wake` runs on a connection thread whenever a
    /// request is queued (an egui `request_repaint`).
    pub fn bind(data_dir: &Path, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(SOCKET_FILE);
        if path.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "socket path {} is longer than {MAX_SOCKET_PATH} bytes; use a shorter data directory",
                    path.display()
                ),
            ));
        }
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        let (tx, requests) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("switchboard-control".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let Ok(stream) = stream else { continue };
                        let tx = tx.clone();
                        let wake = Arc::clone(&wake);
                        let _ = std::thread::Builder::new()
                            .name("switchboard-control-conn".into())
                            .spawn(move || serve_connection(stream, &tx, &*wake));
                    }
                })?
        };
        Ok(Self {
            path,
            requests,
            stop,
            thread: Some(thread),
        })
    }

    /// Every request that arrived since the last poll.
    pub fn poll(&mut self) -> Vec<Incoming> {
        self.requests.try_iter().collect()
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ControlSocket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The accept loop only checks the flag after a connection, so
        // make one to unblock it.
        let _ = UnixStream::connect(&self.path);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// One connection: a line in, a line out, until the client hangs up. A
/// line that is not a request is answered with `failed` and the
/// connection stays up; a reply the app never sends is answered the
/// same way after `ANSWER_TIMEOUT`.
fn serve_connection(stream: UnixStream, tx: &mpsc::Sender<Incoming>, wake: &dyn Fn()) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match Request::parse(&line) {
            Err(e) => Reply::failed(format!("bad request: {e}")),
            Ok(request) => {
                let (reply_tx, reply_rx) = mpsc::channel();
                if tx
                    .send(Incoming {
                        request,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    break;
                }
                wake();
                match reply_rx.recv_timeout(ANSWER_TIMEOUT) {
                    Ok(reply) => reply,
                    Err(_) => Reply::failed(NO_ANSWER),
                }
            }
        };
        if writer.write_all(reply.to_line().as_bytes()).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    fn requested(op: &str) -> OpLine {
        OpLine::Requested {
            op: op.into(),
            kind: "session.new".into(),
            ids: vec!["s1".into()],
            at: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn lines_survive_a_reopen_and_a_torn_tail_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = OperationsLog::open(dir.path()).unwrap();
        log.append(&requested("a")).unwrap();
        log.append(&OpLine::Replied {
            op: "a".into(),
            reply: "{\"reply\":\"launched\"}".into(),
            at: SystemTime::UNIX_EPOCH,
        })
        .unwrap();
        log.append(&requested("b")).unwrap();
        // A crash mid-write leaves a partial line.
        let path = dir.path().join(OPERATIONS_FILE);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"line\":\"requested\",\"op\":\"c")
            .unwrap();
        drop(file);

        let reopened = OperationsLog::open(dir.path()).unwrap();
        assert_eq!(reopened.lines().len(), 3);
        assert_eq!(reopened.find("a").len(), 2);
        assert_eq!(reopened.find("b"), vec![requested("b")]);
        assert!(reopened.find("c").is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn requests_come_off_the_socket_and_replies_go_back_on_the_same_line() {
        use std::sync::atomic::AtomicUsize;
        use switchboard_control::{Body, Client, Made, RecordKind};
        let dir = tempfile::tempdir().unwrap();
        // Keep the path short: a tempdir under a long HOME would overrun.
        let woken = Arc::new(AtomicUsize::new(0));
        let w = Arc::clone(&woken);
        let mut socket = ControlSocket::bind(dir.path(), move || {
            w.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        let path = socket.path().to_path_buf();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let client = std::thread::spawn(move || {
            let mut client = Client::connect(&path).unwrap();
            let first = client.call(&Request::new("op-1", Body::Spaces)).unwrap();
            // A line that is not a request gets `failed`, and the
            // connection is still good for the next one.
            let mut raw = UnixStream::connect(&path).unwrap();
            raw.write_all(b"not json\n").unwrap();
            let mut line = String::new();
            BufReader::new(raw.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let bad = Reply::parse(line.trim_end()).unwrap();
            let second = client.call(&Request::new("op-2", Body::Spaces)).unwrap();
            (first, bad, second)
        });
        let mut served = 0;
        while served < 2 {
            for incoming in socket.poll() {
                served += 1;
                let reply = Reply::Persisted {
                    made: vec![Made {
                        kind: RecordKind::Space,
                        id: incoming.request.op.clone(),
                    }],
                };
                incoming.reply.send(reply).unwrap();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let (first, bad, second) = client.join().unwrap();
        assert_eq!(first.made()[0].id, "op-1");
        assert_eq!(second.made()[0].id, "op-2");
        assert!(matches!(bad, Reply::Failed { reason } if reason.contains("bad request")));
        assert!(woken.load(Ordering::SeqCst) >= 2);
        drop(socket);
        assert!(!dir.path().join(SOCKET_FILE).exists());
    }
}
