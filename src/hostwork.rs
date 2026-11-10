//! Calls to the process host that must not stall the frame: polls and
//! pane writes run here, each on one of two worker threads, and their
//! answers wait on a channel for `logic` to drain. Nothing here touches
//! the core; the app turns each answer into an action that already exists.
//!
//! Two lanes, so a capture of thirty panes never delays a send. Each is
//! one thread reading one FIFO channel: every write goes through a single
//! lane so that order never rests on how tmux treats separate client
//! processes, and no two pastes on one pane's buffer ever overlap.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::core::{Composer, RecordId};
use crate::ports::host::{HostId, HostStatus, ProcessHost};

/// The longest `wait_done` waits for a job that should take milliseconds.
const WAIT_LIMIT: Duration = Duration::from_secs(60);

/// Why a pane is being read, which decides where the text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// A card's caption, and the whole snapshot when it is shown.
    Caption { snapshot: bool },
    /// Whether the pane sits at Claude's trust question.
    Trust,
}

/// One pane to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// The record the text belongs to.
    pub id: RecordId,
    /// The pane to read.
    pub host: HostId,
    /// How many trailing lines to read; `None` reads the whole window.
    pub lines: Option<usize>,
    /// Where the text goes when it arrives.
    pub purpose: Purpose,
}

/// What a pane write sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteKind {
    /// Text then Enter.
    Line(String),
    /// Text then Enter from one of the owner's boxes, whose draft is
    /// settled by the outcome.
    Message {
        id: RecordId,
        text: String,
        from: Composer,
    },
    /// Raw bytes.
    Keys(Vec<u8>),
}

/// One write into a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneWrite {
    /// The pane written to.
    pub host: HostId,
    /// What the attempt is called in a failure notice.
    pub what: String,
    /// The bytes to send and how.
    pub kind: WriteKind,
}

/// A worker's answer to one job.
#[derive(Debug)]
pub enum Done {
    /// A host list, with the generation it was asked for under.
    Listed {
        generation: u64,
        result: Result<Vec<HostStatus>, String>,
    },
    /// The panes of one batch, in the order asked; a pane that could not
    /// be read has an `Err`.
    Captured(Vec<(Capture, Result<String, String>)>),
    /// A pane write's outcome, with the write handed back so the app can
    /// settle the message it carried.
    Wrote {
        write: PaneWrite,
        result: Result<(), String>,
    },
}

enum ReadJob {
    List { generation: u64 },
    Captures(Vec<Capture>),
}

/// The worker threads and what is still out with them. A thread ends when
/// this is dropped, because its channel closes.
pub struct HostWork {
    reads: Sender<ReadJob>,
    writes: Sender<PaneWrite>,
    done: Receiver<Done>,
    /// Filled on the first `logic`; empty in tests, which drain by hand.
    wake: Arc<OnceLock<egui::Context>>,
    list_out: bool,
    captures_out: bool,
    writes_out: usize,
}

impl HostWork {
    /// Start both lanes on `host`. The `Arc` lets the frame and the two
    /// threads share one host; `ProcessHost: Send + Sync` is what makes
    /// that allowed.
    #[must_use]
    pub fn new(host: &Arc<dyn ProcessHost>) -> Self {
        let (reads, read_jobs) = mpsc::channel();
        let (writes, write_jobs) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let wake = Arc::new(OnceLock::new());
        let answers = Answers {
            done: done_tx,
            wake: Arc::clone(&wake),
        };
        let read_host = Arc::clone(host);
        let read_answers = answers.clone();
        std::thread::spawn(move || read_lane(&*read_host, &read_jobs, &read_answers));
        let write_host = Arc::clone(host);
        std::thread::spawn(move || write_lane(&*write_host, &write_jobs, &answers));
        Self {
            reads,
            writes,
            done,
            wake,
            list_out: false,
            captures_out: false,
            writes_out: 0,
        }
    }

    /// Give the workers the context to wake the frame with.
    pub fn wake(&self, ctx: &egui::Context) {
        self.wake.get_or_init(|| ctx.clone());
    }

    /// Queue a host list, unless one is still out: a slow tmux must not
    /// pile up polls. Whether it was queued.
    pub fn list(&mut self, generation: u64) -> bool {
        if self.list_out {
            return false;
        }
        self.list_out = self.reads.send(ReadJob::List { generation }).is_ok();
        self.list_out
    }

    /// Queue a batch of pane reads, unless one is still out. Whether it
    /// was queued.
    pub fn capture(&mut self, batch: Vec<Capture>) -> bool {
        if self.captures_out {
            return false;
        }
        self.captures_out = self.reads.send(ReadJob::Captures(batch)).is_ok();
        self.captures_out
    }

    /// Queue a pane write behind every earlier one. The write back as the
    /// error when the lane has ended and nothing was queued.
    ///
    /// # Errors
    /// Returns the write itself if the write lane's thread is gone.
    pub fn write(&mut self, write: PaneWrite) -> Result<(), PaneWrite> {
        self.writes.send(write).map_err(|e| e.0)?;
        self.writes_out += 1;
        Ok(())
    }

    /// An answer that has already arrived, without waiting.
    pub fn try_done(&mut self) -> Option<Done> {
        let done = self.done.try_recv().ok()?;
        self.mark(&done);
        Some(done)
    }

    /// The next answer, waiting for it; `None` once nothing is out.
    ///
    /// # Panics
    /// If a job is out and no answer comes within `WAIT_LIMIT`, because a
    /// lane died outside `guarded`; hanging would hide that.
    pub fn wait_done(&mut self) -> Option<Done> {
        if !self.list_out && !self.captures_out && self.writes_out == 0 {
            return None;
        }
        let done = self
            .done
            .recv_timeout(WAIT_LIMIT)
            .expect("a host lane stopped answering");
        self.mark(&done);
        Some(done)
    }

    fn mark(&mut self, done: &Done) {
        match done {
            Done::Listed { .. } => self.list_out = false,
            Done::Captured(_) => self.captures_out = false,
            Done::Wrote { .. } => self.writes_out = self.writes_out.saturating_sub(1),
        }
    }
}

/// How a lane answers: a result on the channel, then a nudge to the frame.
#[derive(Clone)]
struct Answers {
    done: Sender<Done>,
    wake: Arc<OnceLock<egui::Context>>,
}

impl Answers {
    fn send(&self, done: Done) {
        let _ = self.done.send(done);
        // After the send, so the pass this wakes finds the result.
        if let Some(ctx) = self.wake.get() {
            ctx.request_repaint();
        }
    }
}

/// Run one host call and report a panic as an error, so a lane survives
/// it and the frame still hears an answer to the job. `AssertUnwindSafe`
/// says nothing borrowed is reused in a broken state: the call only reads
/// its arguments, and a fake's lock recovers or panics again on its own.
fn guarded<T>(call: impl FnOnce() -> std::io::Result<T>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(_) => Err("the host call panicked".to_owned()),
    }
}

fn read_lane(host: &dyn ProcessHost, jobs: &Receiver<ReadJob>, answers: &Answers) {
    for job in jobs {
        answers.send(match job {
            ReadJob::List { generation } => Done::Listed {
                generation,
                result: guarded(|| host.list()),
            },
            ReadJob::Captures(batch) => Done::Captured(
                batch
                    .into_iter()
                    .map(|capture| {
                        let text = guarded(|| host.snapshot(&capture.host, capture.lines));
                        (capture, text)
                    })
                    .collect(),
            ),
        });
    }
}

fn write_lane(host: &dyn ProcessHost, jobs: &Receiver<PaneWrite>, answers: &Answers) {
    for write in jobs {
        let result = guarded(|| match &write.kind {
            WriteKind::Line(text) | WriteKind::Message { text, .. } => {
                host.send_line(&write.host, text)
            }
            WriteKind::Keys(bytes) => host.write(&write.host, bytes),
        });
        answers.send(Done::Wrote { write, result });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::fakes::FakeHost;
    use crate::ports::host::Liveness;

    fn work(fake: &FakeHost) -> HostWork {
        let host: Arc<dyn ProcessHost> = Arc::new(fake.clone());
        HostWork::new(&host)
    }

    fn line(pane: &str, text: &str) -> PaneWrite {
        PaneWrite {
            host: HostId(pane.into()),
            what: format!("send to {pane}"),
            kind: WriteKind::Line(text.into()),
        }
    }

    fn all_done(work: &mut HostWork) -> Vec<Done> {
        std::iter::from_fn(|| work.wait_done()).collect()
    }

    #[test]
    fn writes_run_in_the_order_queued() {
        let fake = FakeHost::default();
        fake.state().delay = Duration::from_millis(2);
        let mut work = work(&fake);
        for i in 0..5 {
            work.write(line("a", &format!("m{i}"))).unwrap();
        }
        assert_eq!(all_done(&mut work).len(), 5);
        let written: Vec<String> = fake
            .state()
            .written
            .iter()
            .map(|(_, t)| t.clone())
            .collect();
        assert_eq!(written, ["m0", "m1", "m2", "m3", "m4"]);
    }

    #[test]
    fn keys_keep_their_place_among_lines() {
        let fake = FakeHost::default();
        let mut work = work(&fake);
        work.write(line("a", "one")).unwrap();
        work.write(PaneWrite {
            host: HostId("a".into()),
            what: "keys".into(),
            kind: WriteKind::Keys(b"\x1b".to_vec()),
        })
        .unwrap();
        work.write(line("a", "two")).unwrap();
        all_done(&mut work);
        let keys: Vec<Vec<u8>> = fake.state().keys.iter().map(|(_, k)| k.clone()).collect();
        assert_eq!(
            keys,
            [
                b"one".to_vec(),
                b"\r".to_vec(),
                b"\x1b".to_vec(),
                b"two".to_vec(),
                b"\r".to_vec()
            ]
        );
    }

    #[test]
    fn a_failing_write_answers_once_and_later_writes_still_run() {
        let fake = FakeHost::default();
        fake.state().fail_write = Some("pane is dead".into());
        let mut work = work(&fake);
        work.write(line("a", "lost")).unwrap();
        let Some(Done::Wrote { write, result }) = work.wait_done() else {
            panic!("no answer");
        };
        assert_eq!(write.what, "send to a");
        assert_eq!(result, Err("pane is dead".to_owned()));
        assert!(work.wait_done().is_none(), "one answer");
        fake.state().fail_write = None;
        work.write(line("a", "kept")).unwrap();
        all_done(&mut work);
        assert_eq!(fake.state().written.len(), 1);
    }

    #[test]
    fn a_second_list_is_not_queued_while_one_is_out() {
        let fake = FakeHost::default();
        fake.state().delay = Duration::from_millis(20);
        let mut work = work(&fake);
        assert!(work.list(1));
        assert!(!work.list(2), "one is out");
        assert!(work.capture(Vec::new()));
        assert!(!work.capture(Vec::new()), "a batch is out");
        assert_eq!(all_done(&mut work).len(), 2);
        assert!(work.list(3), "the lane is free again");
        all_done(&mut work);
    }

    #[test]
    fn reads_answer_in_request_order() {
        let fake = FakeHost::default();
        fake.state().delay = Duration::from_millis(2);
        fake.state().statuses.push(HostStatus {
            id: HostId("a".into()),
            liveness: Liveness::Running {
                pid: 1,
                command: "sh".into(),
            },
            cwd: None,
            last_activity: None,
            title: None,
        });
        fake.state()
            .snapshots
            .insert(HostId("a".into()), "screen".into());
        let mut work = work(&fake);
        let id = RecordId(uuid::Uuid::new_v4());
        work.capture(vec![Capture {
            id,
            host: HostId("a".into()),
            lines: Some(3),
            purpose: Purpose::Trust,
        }]);
        work.list(7);
        let answers = all_done(&mut work);
        let [Done::Captured(panes), Done::Listed { generation, result }] = &answers[..] else {
            panic!("out of order: {answers:?}");
        };
        assert_eq!(panes[0].1.as_deref(), Ok("screen"));
        assert_eq!(*generation, 7);
        assert_eq!(result.as_ref().map(Vec::len), Ok(1));
    }

    #[test]
    fn a_panicking_read_still_answers_and_the_lane_survives() {
        let fake = FakeHost::default();
        fake.state().panic_snapshot = true;
        let mut work = work(&fake);
        work.capture(vec![Capture {
            id: RecordId(uuid::Uuid::new_v4()),
            host: HostId("a".into()),
            lines: None,
            purpose: Purpose::Trust,
        }]);
        let Some(Done::Captured(panes)) = work.wait_done() else {
            panic!("no answer");
        };
        assert!(panes[0].1.is_err());
        assert!(work.list(1));
        assert!(matches!(work.wait_done(), Some(Done::Listed { .. })));
    }

    #[test]
    fn a_write_to_an_ended_lane_comes_back_unqueued() {
        let fake = FakeHost::default();
        let mut work = work(&fake);
        let (dead, _) = mpsc::channel();
        work.writes = dead;
        let back = work.write(line("a", "x")).unwrap_err();
        assert_eq!(back, line("a", "x"));
        assert!(work.wait_done().is_none(), "nothing is out");
    }
}
