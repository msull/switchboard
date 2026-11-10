//! One event subscription per host (`GET /api/v1/events`, a pinned
//! WebSocket), opened on first use and shared by every `wait`, `run`
//! and `ask_supervisor` in this process, so a host's cap on paired
//! connections and its per-subscriber queue are spent once, not per
//! call. A reader thread keeps the frames it saw, numbered; a waiter
//! notes the number before it reads the state it compares against, so
//! no frame falls between its read and its wait. A feed nobody waited
//! on for `IDLE` closes itself, and a closed one is opened again on the
//! next use.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest as _;

use crate::config::Host;
use crate::http;

/// Frames kept per feed for waiters that are behind.
const KEPT: usize = 512;
/// A feed with no waiter for this long closes its socket.
pub const IDLE: Duration = Duration::from_secs(300);
/// How often the reader looks up from the socket.
const TICK: Duration = Duration::from_millis(500);

/// A host's place in the hub: its feed once opened.
type Slot = Arc<Mutex<Option<Arc<Feed>>>>;

/// Every host's feed.
#[derive(Default)]
pub struct Hub {
    /// One slot per host. The map's lock is held only to find a slot;
    /// the slot's own lock is held while its feed opens, so a dead host
    /// stalls callers of that host alone, and two callers of one host
    /// open one socket.
    feeds: Mutex<HashMap<String, Slot>>,
    opened: AtomicUsize,
}

impl Hub {
    /// The live feed of `host`, opened now if it has none.
    pub fn feed(&self, host: &Host) -> Result<Arc<Feed>, http::Failure> {
        let slot = Arc::clone(
            self.feeds
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(host.name.clone())
                .or_default(),
        );
        let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(f) = slot.as_ref()
            && f.alive()
        {
            f.touch();
            return Ok(Arc::clone(f));
        }
        let feed = Feed::open(host)?;
        self.opened.fetch_add(1, Ordering::SeqCst);
        *slot = Some(Arc::clone(&feed));
        Ok(feed)
    }

    /// How many sockets this hub has opened, for the tests.
    #[must_use]
    pub fn opened(&self) -> usize {
        self.opened.load(Ordering::SeqCst)
    }
}

struct State {
    /// The number of the last frame seen.
    seq: u64,
    frames: VecDeque<(u64, serde_json::Value)>,
    alive: bool,
    error: Option<String>,
    used: Instant,
}

/// One host's subscription.
pub struct Feed {
    state: Mutex<State>,
    cv: Condvar,
}

/// How a wait ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Waited {
    Matched(serde_json::Value),
    TimedOut,
    Cancelled,
    /// The socket closed; the reason.
    Closed(String),
}

impl Feed {
    fn open(host: &Host) -> Result<Arc<Self>, http::Failure> {
        let tls = http::connect(&host.addresses, &host.spki_sha256, http::TIMEOUT)?;
        let mut request = "wss://localhost/api/v1/events"
            .into_client_request()
            .map_err(|e| http::Failure::Io(e.to_string()))?;
        let bearer = format!("Bearer {}", host.token)
            .parse()
            .map_err(|_| http::Failure::Io("the token is not a header value".into()))?;
        request.headers_mut().insert("Authorization", bearer);
        let (mut ws, _) = tungstenite::client(request, tls).map_err(|e| match e {
            tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r))
                if r.status() == 401 =>
            {
                http::Failure::Io("unauthorized".into())
            }
            e => http::Failure::Io(format!("event stream: {e}")),
        })?;
        ws.get_mut()
            .sock
            .set_read_timeout(Some(TICK))
            .map_err(|e| http::Failure::Io(e.to_string()))?;
        let feed = Arc::new(Self {
            state: Mutex::new(State {
                seq: 0,
                frames: VecDeque::new(),
                alive: true,
                error: None,
                used: Instant::now(),
            }),
            cv: Condvar::new(),
        });
        let reader = Arc::clone(&feed);
        std::thread::spawn(move || reader.read(&mut ws));
        Ok(feed)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn alive(&self) -> bool {
        self.lock().alive
    }

    fn touch(&self) {
        self.lock().used = Instant::now();
    }

    fn read(&self, ws: &mut tungstenite::WebSocket<http::Tls>) {
        let why = loop {
            if self.lock().used.elapsed() > IDLE {
                let _ = ws.close(None);
                break "idle".to_owned();
            }
            match ws.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    let value =
                        serde_json::from_str(text.as_str()).unwrap_or(serde_json::Value::Null);
                    let mut s = self.lock();
                    s.seq += 1;
                    let seq = s.seq;
                    s.frames.push_back((seq, value));
                    while s.frames.len() > KEPT {
                        s.frames.pop_front();
                    }
                    drop(s);
                    self.cv.notify_all();
                }
                Ok(tungstenite::Message::Close(_)) => break "closed by the host".to_owned(),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => break e.to_string(),
            }
        };
        let mut s = self.lock();
        s.alive = false;
        s.error = Some(why);
        drop(s);
        self.cv.notify_all();
    }

    /// The number of the last frame seen: what a waiter starts after.
    #[must_use]
    pub fn mark(&self) -> u64 {
        let mut s = self.lock();
        s.used = Instant::now();
        s.seq
    }

    /// The first frame after `since` that `matches`, or a resync, by
    /// `deadline`. `since` moves past every frame looked at, so a caller
    /// that waits again goes on from there.
    pub fn wait_for(
        &self,
        since: &mut u64,
        deadline: Instant,
        cancel: &AtomicBool,
        matches: &dyn Fn(&serde_json::Value) -> bool,
    ) -> Waited {
        let mut s = self.lock();
        loop {
            s.used = Instant::now();
            let from = *since;
            for (seq, frame) in s.frames.iter().filter(|(seq, _)| *seq > from) {
                *since = *seq;
                if frame["type"] == "resync_required" || matches(frame) {
                    return Waited::Matched(frame.clone());
                }
            }
            *since = (*since).max(s.seq);
            if cancel.load(Ordering::SeqCst) {
                return Waited::Cancelled;
            }
            if !s.alive {
                return Waited::Closed(s.error.clone().unwrap_or_default());
            }
            let now = Instant::now();
            if now >= deadline {
                return Waited::TimedOut;
            }
            // Woken by a frame, or every tick to look at the cancel.
            let step = (deadline - now).min(TICK);
            s = self
                .cv
                .wait_timeout(s, step)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// Whether `frame` is a change to the resource `kind` with `id`.
#[must_use]
pub fn is_change(frame: &serde_json::Value, kinds: &[&str], id: &str) -> bool {
    matches!(frame["type"].as_str(), Some("changed" | "removed"))
        && kinds
            .iter()
            .any(|k| frame["resource"]["kind"].as_str() == Some(k))
        && frame["resource"]["id"].as_str() == Some(id)
}
