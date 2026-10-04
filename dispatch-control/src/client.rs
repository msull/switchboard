//! A blocking client: one connection, one request at a time, one reply
//! per request: what the app uses to call a runner. Std only, so the
//! app and tests need nothing else.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::{Reply, Request};

/// How long a reply may take. A take fetches the issue from its source
/// first, so this is generous.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    /// Connect to the socket at `path` (`<Dispatch data dir>/dispatch.sock`).
    pub fn connect(path: &Path) -> io::Result<Self> {
        Self::connect_with_timeout(path, REPLY_TIMEOUT)
    }

    /// The same, with replies (and writes) allowed `timeout` rather than
    /// the generous default: a health check wants to know quickly.
    pub fn connect_with_timeout(path: &Path, timeout: Duration) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let writer = stream.try_clone()?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    /// Send one request and wait for its reply. A closed socket or a
    /// reply that does not parse is an error; a `Reply::Failed` is not,
    /// since the runner did answer.
    pub fn call(&mut self, request: &Request) -> io::Result<Reply> {
        self.writer.write_all(request.to_line().as_bytes())?;
        self.writer.flush()?;
        let mut line = String::new();
        let n = self.reader.read_line(&mut line)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the dispatch socket closed before replying",
            ));
        }
        Reply::parse(line.trim_end()).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bad reply from the dispatch socket: {e}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    use super::*;
    use crate::Body;

    #[test]
    fn a_call_is_one_line_out_and_one_line_back() {
        let dir = std::env::temp_dir().join(format!("dcc-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("s.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let req = Request::parse(line.trim_end()).unwrap();
            let reply = Reply::Artifact { text: req.op };
            let mut w = stream;
            w.write_all(reply.to_line().as_bytes()).unwrap();
        });
        let mut client = Client::connect(&path).unwrap();
        let reply = client.call(&Request::new("op-9", Body::Status)).unwrap();
        assert_eq!(
            reply,
            Reply::Artifact {
                text: "op-9".into()
            }
        );
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
