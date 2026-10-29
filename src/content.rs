//! What the app read from outside the core and keeps between frames,
//! with the version it was read at: the parsed transcripts, and the
//! content the remote port serves through one worker thread (a pane's
//! screen, a conversation and the last response read from it, a run's
//! output, a file, and the text of Dispatch's tickets). The views read
//! the transcript cache through `DrawCtx`, and the remote port reads
//! its own masked copy on the worker, so a transcript is parsed once
//! per change for each.
//! Nothing here is persisted or read by the core.
//!
//! The worker does the read, the mask, the version and the
//! serialization off the frame, one job at a time, and reports what it
//! observed; the frame keeps the versions, assigns revisions and emits
//! `changed`. The same worker polls every watched key every
//! [`WATCH_POLL`], so a watched pane or transcript moves for a device
//! while the host shows something else or is hidden.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use crate::adapters::files::{self, FileError};
use crate::adapters::remote::RemoteReply;
use crate::adapters::scrollback;
use crate::core::file_refs;
use crate::core::model::{EnvSet, EnvVar, ProjectEnv};
use crate::core::{ProjectId, RecordId, ResumeHandle, resolve_sets};
use crate::ports::host::{HostId, ProcessHost};
use crate::ports::secrets::SecretStore;
use crate::ports::transcript::{
    ActivityKind, Conversation, TranscriptReader, TranscriptVersion, Turn,
};
use crate::sync::lock_recover;

/// How often the worker reads every watched key.
pub const WATCH_POLL: Duration = Duration::from_secs(2);
/// A key with no subscriber and no read for this long leaves the set.
pub const WATCH_IDLE: Duration = Duration::from_secs(60);
/// The most keys the set holds; past it the least recently read goes.
pub const WATCH_CAP: usize = 128;
/// A screen's or a run output's text after masking is cut from the
/// top to this.
pub const SCREEN_BYTES_CAP: usize = 256 * 1024;
/// How many lines above the asked-for ones a screen read takes and
/// masks with them, so a value wrapped across the window's first line
/// is still found whole.
pub const SCREEN_SLACK_LINES: usize = 8;

/// The most lines a screen read returns.
pub const SCREEN_LINES_CAP: usize = 200;
/// The most lines a run output read returns.
pub const RUN_LINES_CAP: usize = 500;
/// One turn's JSON is cut to this: the final text kept where possible,
/// the tool detail and the notes dropped first.
pub const TURN_BYTES_CAP: usize = 256 * 1024;
/// A conversation page stops before its turns' JSON passes this.
pub const PAGE_BYTES_CAP: usize = 640 * 1024;
/// The most of a tool call's input, its result, or an activity's text
/// a page carries; a turn read on its own carries them whole.
pub const ACTIVITY_BYTES_CAP: usize = 512;
/// The most turns a page carries, and how many without `limit`.
pub const PAGE_TURNS_CAP: usize = 100;
pub const PAGE_TURNS_DEFAULT: usize = 20;
/// A pinned file is sent whole or not at all: more source bytes than
/// this answer `too_large`. It is Dispatch's bound on a ticket file on
/// purpose, so a file Dispatch reads is never refused again here.
pub const FILE_BYTES_CAP: u64 = dispatch_control::FILE_TEXT_MAX;
/// How many jobs may wait for the worker.
const JOBS: usize = 64;

/// One piece of content a device can read and watch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContentKey {
    Screen(RecordId),
    /// A Claude Code session's transcript; the last response is a
    /// projection of it and shares its version.
    Conversation(RecordId),
    /// One run's log, by the record and the run number.
    RunOutput(RecordId, u32),
    /// A file some working set pins, by its project and its path under
    /// the root.
    File(ProjectId, PathBuf),
    /// Text from Dispatch's tickets, by the ticket (`None` for the
    /// list) and the part: masked on the worker, never watched.
    TicketText(Option<String>, TicketPart),
}

/// Which of a ticket's texts a [`ContentKey::TicketText`] read is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TicketPart {
    /// Every ticket as the list shows it.
    List,
    /// One ticket's summary, body, attempts and lanes.
    Detail,
    /// One ticket's events.
    Events,
    /// One file of an attempt, by its key.
    File(String),
}

impl ContentKey {
    /// The `kind` on the wire.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Screen(_) => "screen",
            Self::Conversation(_) => "conversation",
            Self::RunOutput(..) => "run_output",
            Self::File(..) => "file",
            Self::TicketText(..) => "ticket",
        }
    }

    /// The `id` on the wire: the record, `record/n` for a run, or
    /// `project/path` for a file.
    #[must_use]
    pub fn id(&self) -> String {
        match self {
            Self::Screen(id) | Self::Conversation(id) => id.0.to_string(),
            Self::RunOutput(id, n) => format!("{}/{n}", id.0),
            Self::File(project, path) => format!("{}/{}", project.0, path.display()),
            Self::TicketText(ticket, part) => {
                let ticket = ticket.as_deref().unwrap_or("");
                match part {
                    TicketPart::List => "list".to_owned(),
                    TicketPart::Detail => ticket.to_owned(),
                    TicketPart::Events => format!("{ticket}/events"),
                    TicketPart::File(key) => format!("{ticket}/files/{key}"),
                }
            }
        }
    }

    /// The key a `watch` frame names, if this host serves the kind.
    #[must_use]
    pub fn parse(kind: &str, id: &str) -> Option<Self> {
        let record = |s: &str| uuid::Uuid::parse_str(s).ok().map(RecordId);
        match kind {
            "screen" => record(id).map(Self::Screen),
            "conversation" => record(id).map(Self::Conversation),
            "run_output" => {
                let (rec, n) = id.split_once('/')?;
                Some(Self::RunOutput(record(rec)?, n.parse().ok()?))
            }
            "file" => {
                let (project, path) = id.split_once('/')?;
                let project = ProjectId(uuid::Uuid::parse_str(project).ok()?);
                Some(Self::File(project, PathBuf::from(path)))
            }
            _ => None,
        }
    }

    /// The session a key reads from; a file has none.
    #[must_use]
    pub fn record(&self) -> Option<RecordId> {
        match self {
            Self::Screen(id) | Self::Conversation(id) | Self::RunOutput(id, _) => Some(*id),
            Self::File(..) | Self::TicketText(..) => None,
        }
    }
}

/// What a policy is resolved from, gathered on the frame from the
/// core: the project's environment and the sets it and the session
/// may use. The values come from the secret store on the worker, so a
/// slow Keychain call never costs a frame. `epoch` counts the saves
/// and secret changes the frame saw, so a value changed under an
/// unchanged name resolves again.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PolicySpec {
    pub epoch: u64,
    pub project: ProjectId,
    pub root: PathBuf,
    pub global: Vec<EnvVar>,
    pub project_env: ProjectEnv,
    pub sets: Vec<EnvSet>,
    pub project_sets: Vec<String>,
    pub record_sets: Vec<String>,
}

impl PolicySpec {
    /// Names this spec in the worker's cache.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        serde_json::to_string(self).unwrap_or_default().hash(&mut h);
        h.finish()
    }
}

/// The policy a spec names: the project's secret variables and its
/// granted sets' secrets, read from dotenv files and the store. A
/// store that fails is an error, never an unmasked text.
pub fn resolve_policy(spec: &PolicySpec, secrets: &dyn SecretStore) -> Result<Policy, String> {
    let mut dotenv = Vec::new();
    if spec.project_env.load_dotenv {
        for file in spec.project_env.files() {
            match std::fs::read_to_string(spec.root.join(&file)) {
                Ok(text) => dotenv.push((file, crate::adapters::dotenv::parse(&text))),
                Err(e) => log::warn!("{file} in {}: {e}", spec.root.display()),
            }
        }
    }
    let example = std::fs::read_to_string(spec.root.join(".env.example"))
        .map(|t| crate::adapters::dotenv::names(&t))
        .unwrap_or_default();
    // `lookup` is a `Fn` closure, which may only borrow what it
    // captures shared; the `RefCell` lets it still note the first
    // store error, so a set's secret that cannot be read fails the
    // policy instead of resolving as missing.
    let lookup_failed: RefCell<Option<String>> = RefCell::new(None);
    let lookup = |account: &str| match secrets.get(account) {
        Ok(value) => value,
        Err(e) => {
            log::warn!("secret {account}: {e}");
            lookup_failed.borrow_mut().get_or_insert(e);
            None
        }
    };
    let resolved = crate::core::env::resolve(
        &spec.global,
        spec.project,
        &spec.project_env,
        &dotenv,
        &example,
        &lookup,
    );
    let mut values: Vec<(String, String)> = resolved
        .vars
        .into_iter()
        .filter(|v| v.secret)
        .filter_map(|v| v.value.filter(|x| !x.is_empty()).map(|x| (v.name, x)))
        .collect();
    // A child of `switchboard-env exec` may print a set's secret too.
    let sets = resolve_sets(&spec.sets, &spec.project_sets, &spec.record_sets, &lookup)?;
    let secret_names: Vec<&str> = spec
        .sets
        .iter()
        .flat_map(|s| s.vars.iter().filter(|v| v.secret).map(|v| v.name.as_str()))
        .collect();
    values.extend(
        sets.pairs
            .into_iter()
            .filter(|(name, value)| !value.is_empty() && secret_names.contains(&name.as_str())),
    );
    // A store that answered with an error for any secret account is a
    // failure of the whole policy: nothing is sent unredacted.
    let mut failed = lookup_failed.into_inner();
    for account in spec
        .project_env
        .vars
        .iter()
        .filter(|v| v.secret)
        .map(|v| crate::core::SecretScope::Project(spec.project).account(&v.name))
    {
        if let Err(e) = secrets.get(&account) {
            failed.get_or_insert(e);
        }
    }
    if let Some(e) = failed {
        return Err(e);
    }
    Ok(Policy::new(values))
}

/// The secret values a text leaving the port is masked of, and the
/// fingerprint that names this policy in every version. The values are
/// in memory only: never logged, never stored.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub secrets: Vec<(String, String)>,
    pub fingerprint: u64,
}

impl Policy {
    #[must_use]
    pub fn new(mut secrets: Vec<(String, String)>) -> Self {
        secrets.sort();
        secrets.dedup();
        let mut h = DefaultHasher::new();
        secrets.hash(&mut h);
        Self {
            fingerprint: h.finish(),
            secrets,
        }
    }

    /// The longest value, which is how far a windowed read reaches
    /// back so a value on the window's edge is masked whole.
    #[must_use]
    pub fn overlap(&self) -> usize {
        self.secrets.iter().map(|(_, v)| v.len()).max().unwrap_or(0)
    }

    fn mask(&self, text: &str) -> String {
        redact(text, &self.secrets)
    }

    /// One policy masking every value of `policies`.
    fn union(policies: Vec<Policy>) -> Self {
        Self::new(policies.into_iter().flat_map(|p| p.secrets).collect())
    }
}

/// `value` with every string in it masked, before it is serialized, so
/// a secret holding a quote, a backslash or a control character is
/// found as it is rather than as JSON escapes it. Object keys are the
/// view's own field names and are left alone.
fn mask_json(value: &mut serde_json::Value, policy: &Policy) {
    match value {
        serde_json::Value::String(s) => *s = policy.mask(s),
        serde_json::Value::Array(items) => {
            for item in items {
                mask_json(item, policy);
            }
        }
        serde_json::Value::Object(map) => {
            for field in map.values_mut() {
                mask_json(field, policy);
            }
        }
        _ => {}
    }
}

/// Where a job's content comes from, resolved on the frame from the
/// record so the worker touches no core state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Pane(HostId),
    /// A Claude Code transcript, with the project root and the
    /// session's working directory the paths in its prose are resolved
    /// against for the links on the wire.
    Transcript {
        handle: ResumeHandle,
        root: PathBuf,
        cwd: PathBuf,
    },
    Log(PathBuf),
    /// A pinned file: the project root and the path under it.
    File {
        root: PathBuf,
        relative: PathBuf,
    },
    /// A view the frame already holds, to be masked whole: a ticket's
    /// text, as the reply carries it.
    Json(serde_json::Value),
}

/// Where a page is anchored. A page anchored at its start runs
/// forward and stops at `limit` or the byte cap, saying `next`; a
/// page anchored at its end runs backward from there, so it always
/// holds the anchor and `from` is wherever it filled up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFrom {
    /// Starts at a 1-based turn number.
    Turn(usize),
    /// Ends at the last turn.
    Last,
    /// Ends at the turn before this 1-based turn number.
    Before(usize),
}

/// What of the content a job wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentView {
    Screen {
        lines: usize,
    },
    Page {
        from: PageFrom,
        limit: usize,
    },
    LastResponse,
    /// One turn whole, with its tool detail uncut.
    Turn {
        n: usize,
    },
    Tail {
        lines: usize,
    },
    /// A file whole.
    Whole,
}

/// One read for the worker: what to read, what names the policy to
/// mask it with, and where the reply goes (nowhere for a poll).
/// `policies` are all masked: usually one, several for text any of
/// several projects' children could have written; any of them failing
/// to resolve refuses the read, and none refuses it too.
#[derive(Clone)]
pub struct ContentJob {
    pub key: ContentKey,
    pub source: Source,
    pub view: ContentView,
    pub policies: Vec<PolicySpec>,
    pub reply: Option<SyncSender<RemoteReply>>,
}

/// Why the worker could not read, as the reply names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentError {
    /// The source could not be read: `failed`, with the reason.
    Failed(String),
    /// The policy could not be resolved: `policy`; nothing is sent.
    Policy(String),
    /// A file over the limit: `too_large`, with its size.
    TooLarge(u64),
}

impl ContentError {
    /// The status, code and reason of the refusal.
    #[must_use]
    pub fn refusal(&self) -> (u16, &'static str, String) {
        match self {
            Self::Failed(reason) => (400, "failed", reason.clone()),
            Self::Policy(_) => (
                503,
                "policy",
                "the secret set could not be resolved; nothing is sent unredacted".into(),
            ),
            Self::TooLarge(size) => (
                413,
                "too_large",
                format!("the file is {size} bytes; the limit is {FILE_BYTES_CAP}"),
            ),
        }
    }
}

/// What the worker saw for a job: the source version and the masked
/// body, or why it could not read; `policy` is the fingerprint of the
/// policy it masked with, which the version carries.
pub struct Observed {
    pub key: ContentKey,
    pub policy: u64,
    pub result: Result<(u64, serde_json::Value), ContentError>,
    pub reply: Option<SyncSender<RemoteReply>>,
}

/// The version the frame holds per key: what it last served and the
/// revision it assigned when that changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentVersion {
    pub source: u64,
    pub policy: u64,
    pub revision: u64,
}

/// A watched key: who asked, and when it was last read if a read put
/// it here. A read keeps it for [`WATCH_IDLE`]; a subscriber keeps it
/// while it holds the watch.
#[derive(Debug, Clone)]
pub struct Watch {
    pub last_read: Option<Instant>,
    pub subscribers: std::collections::HashSet<u64>,
}

/// What goes down the worker's channel: a read, or a word that the
/// poll interval moved.
enum Work {
    Job(Box<ContentJob>),
    Nudge,
}

/// The one thread that reads content for the port.
pub struct ContentWorker {
    jobs: SyncSender<Work>,
    watch: Arc<Mutex<Vec<ContentJob>>>,
    poll: Arc<Mutex<Duration>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ContentWorker {
    /// Start the worker over `host` and `transcripts`; every
    /// observation goes to `observed` and `wake` runs after each so the
    /// frame picks it up.
    #[must_use]
    pub fn spawn(
        host: Arc<dyn ProcessHost>,
        transcripts: Arc<dyn TranscriptReader>,
        secrets: Arc<dyn SecretStore>,
        observed: Sender<Observed>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (jobs, inbox) = mpsc::sync_channel(JOBS);
        let watch: Arc<Mutex<Vec<ContentJob>>> = Arc::new(Mutex::new(Vec::new()));
        let poll = Arc::new(Mutex::new(WATCH_POLL));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (watch, poll, stop) = (Arc::clone(&watch), Arc::clone(&poll), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("switchboard-content".into())
                .spawn(move || {
                    let mut cache = Cache::new(secrets);
                    let every = || *lock_recover(&poll, "content poll");
                    let mut next_poll = Instant::now() + every();
                    while !stop.load(Ordering::SeqCst) {
                        let wait = next_poll.saturating_duration_since(Instant::now());
                        match inbox.recv_timeout(wait) {
                            Ok(Work::Job(job)) => {
                                let out = observe_caught(&*host, &*transcripts, &mut cache, &job);
                                if observed.send(out).is_err() {
                                    return;
                                }
                                wake();
                            }
                            Ok(Work::Nudge) => next_poll = Instant::now() + every(),
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                let jobs = lock_recover(&watch, "content watch").clone();
                                cache.retain(&jobs);
                                for job in jobs {
                                    let out =
                                        observe_caught(&*host, &*transcripts, &mut cache, &job);
                                    if observed.send(out).is_err() {
                                        return;
                                    }
                                }
                                wake();
                                next_poll = Instant::now() + every();
                            }
                        }
                    }
                })
                .ok()
        };
        Self {
            jobs,
            watch,
            poll,
            stop,
            thread,
        }
    }

    /// Queue one read; false when the worker is too far behind, and
    /// the job is dropped.
    #[must_use]
    pub fn submit(&self, job: ContentJob) -> bool {
        self.jobs.try_send(Work::Job(Box::new(job))).is_ok()
    }

    /// Replace what the poll reads.
    pub fn set_watch(&self, jobs: Vec<ContentJob>) {
        *lock_recover(&self.watch, "content watch (set)") = jobs;
    }

    /// How often the poll runs, from now; tests shorten it.
    pub fn set_poll(&self, every: Duration) {
        *lock_recover(&self.poll, "content poll (set)") = every;
        let _ = self.jobs.try_send(Work::Nudge);
    }
}

impl Drop for ContentWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // An empty job wakes the loop so it sees the flag.
        if let Some(t) = self.thread.take() {
            drop(std::mem::replace(&mut self.jobs, mpsc::sync_channel(1).0));
            let _ = t.join();
        }
    }
}

/// What the worker keeps between jobs: the policies resolved per
/// spec, the masked transcripts per record, and the masked files and
/// ticket texts per key, each with the version it was made under, so a
/// poll is a stat and a read of an unchanged source is served from
/// here. Entries no longer watched or read are dropped at the next
/// poll.
struct Cache {
    secrets: Arc<dyn SecretStore>,
    policies: HashMap<u64, (Policy, Instant)>,
    transcripts: HashMap<RecordId, ParsedEntry>,
    files: HashMap<ContentKey, FileEntry>,
    tickets: HashMap<ContentKey, TextEntry>,
}

impl Cache {
    fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            secrets,
            policies: HashMap::new(),
            transcripts: HashMap::new(),
            files: HashMap::new(),
            tickets: HashMap::new(),
        }
    }

    /// Drop every entry, keeping the secret store.
    fn reset(&mut self) {
        self.policies.clear();
        self.transcripts.clear();
        self.files.clear();
        self.tickets.clear();
    }
}

/// A ticket text as masked, under the hash of what the frame sent and
/// the policy it was masked with.
struct TextEntry {
    input: u64,
    policy: u64,
    body: serde_json::Value,
    touched: Instant,
}

struct ParsedEntry {
    handle: ResumeHandle,
    version: TranscriptVersion,
    policy: u64,
    conversation: Conversation,
    links: Links,
    touched: Instant,
}

/// A file an agent named in its prose, as the wire carries it: the
/// byte range of the token in the text as sent, the path under the
/// project root, and the line the token named.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub path: String,
    pub line: Option<u32>,
}

/// The links of every piece of prose in a conversation, by
/// [`file_refs::text_key`] of the text. Existence checks on disk are
/// what the scan costs, so a text already scanned is not scanned
/// again when the transcript grows.
pub type Links = HashMap<u64, Vec<Link>>;

/// The links of `conversation`'s prose, resolved as the app's own
/// view resolves them (the project root, then the session's working
/// directory), kept only for files under the root, since that is what
/// the port serves; entries of `previous` are reused by text.
#[must_use]
pub fn link_map(
    conversation: &Conversation,
    root: &std::path::Path,
    cwd: &std::path::Path,
    previous: &Links,
) -> Links {
    let mut out = Links::new();
    for text in file_refs::prose_texts(conversation) {
        let key = file_refs::text_key(text);
        if out.contains_key(&key) {
            continue;
        }
        let links = previous.get(&key).cloned().unwrap_or_else(|| {
            file_refs::resolve(text, |r| files::existing_file(&r.path, root, cwd))
                .into_iter()
                .filter_map(|(r, path)| {
                    let relative = path.strip_prefix(root).ok()?;
                    Some(Link {
                        start: r.range.start,
                        end: r.range.end,
                        path: relative.to_string_lossy().into_owned(),
                        line: r.line,
                    })
                })
                .collect()
        });
        out.insert(key, links);
    }
    out
}

struct FileEntry {
    version: u64,
    policy: u64,
    body: serde_json::Value,
    touched: Instant,
}

impl Cache {
    /// The policy `spec` names, resolved once per spec.
    fn policy(&mut self, spec: &PolicySpec) -> Result<Policy, ContentError> {
        let key = spec.fingerprint();
        if let Some((policy, touched)) = self.policies.get_mut(&key) {
            *touched = Instant::now();
            return Ok(policy.clone());
        }
        let policy = resolve_policy(spec, &*self.secrets).map_err(ContentError::Policy)?;
        self.policies.insert(key, (policy.clone(), Instant::now()));
        Ok(policy)
    }

    /// The policy a job masks with: the union of its specs', each
    /// resolved once per spec.
    fn job_policy(&mut self, job: &ContentJob) -> Result<Policy, ContentError> {
        let mut all = Vec::new();
        for spec in &job.policies {
            all.push(self.policy(spec)?);
        }
        match all.len() {
            0 => Err(ContentError::Policy("the job names no policy".into())),
            1 => Ok(all.remove(0)),
            _ => Ok(Policy::union(all)),
        }
    }

    /// A ticket's text under `key` masked: from here while what the
    /// frame sent and the policy are both unchanged. A file's text over
    /// [`FILE_BYTES_CAP`] is refused rather than masked and cut.
    fn ticket_text(
        &mut self,
        key: &ContentKey,
        value: &serde_json::Value,
        policy: &Policy,
    ) -> Result<(u64, serde_json::Value), ContentError> {
        if let ContentKey::TicketText(_, TicketPart::File(_)) = key {
            let len = value["text"].as_str().map_or(0, str::len) as u64;
            if len > FILE_BYTES_CAP {
                return Err(ContentError::TooLarge(len));
            }
        }
        let mut h = DefaultHasher::new();
        value.to_string().hash(&mut h);
        let input = h.finish();
        if let Some(entry) = self.tickets.get_mut(key)
            && entry.input == input
            && entry.policy == policy.fingerprint
        {
            entry.touched = Instant::now();
            return Ok((input, entry.body.clone()));
        }
        let mut body = value.clone();
        mask_json(&mut body, policy);
        self.tickets.insert(
            key.clone(),
            TextEntry {
                input,
                policy: policy.fingerprint,
                body: body.clone(),
                touched: Instant::now(),
            },
        );
        Ok((input, body))
    }

    /// The masked conversation for `record` under `handle`, read only
    /// when the file, the handle or the policy moved since.
    fn conversation(
        &mut self,
        transcripts: &dyn TranscriptReader,
        record: RecordId,
        handle: &ResumeHandle,
        root: &std::path::Path,
        cwd: &std::path::Path,
        policy: &Policy,
    ) -> Result<(u64, &Conversation, &Links), ContentError> {
        let version = transcripts
            .version(handle)
            .ok_or_else(|| ContentError::Failed("no transcript file yet".to_owned()))?;
        let current = self.transcripts.get(&record).is_some_and(|e| {
            &e.handle == handle && e.version == version && e.policy == policy.fingerprint
        });
        if !current {
            let conversation = transcripts
                .read_masked(handle, &|s| policy.mask(s))
                .map_err(ContentError::Failed)?;
            let previous = self.transcripts.remove(&record).map(|e| e.links);
            let links = link_map(&conversation, root, cwd, &previous.unwrap_or_default());
            self.transcripts.insert(
                record,
                ParsedEntry {
                    handle: handle.clone(),
                    version,
                    policy: policy.fingerprint,
                    conversation,
                    links,
                    touched: Instant::now(),
                },
            );
        }
        let entry = self.transcripts.get_mut(&record).expect("just inserted");
        entry.touched = Instant::now();
        let mut h = DefaultHasher::new();
        (handle.provider_id(), version).hash(&mut h);
        Ok((h.finish(), &entry.conversation, &entry.links))
    }

    /// The masked file under `key`: the walk and the stat every time,
    /// the read only when the file or the policy moved since.
    fn file(
        &mut self,
        key: &ContentKey,
        root: &std::path::Path,
        relative: &std::path::Path,
        policy: &Policy,
    ) -> Result<(u64, serde_json::Value), ContentError> {
        let opened = files::open_pinned(root, relative).map_err(file_error)?;
        let version = opened.version;
        if let Some(entry) = self.files.get_mut(key)
            && entry.version == version
            && entry.policy == policy.fingerprint
        {
            entry.touched = Instant::now();
            return Ok((version, entry.body.clone()));
        }
        let size = opened.size;
        let modified_ms = opened.modified_ms;
        let text = opened.read(FILE_BYTES_CAP).map_err(file_error)?;
        let body = serde_json::json!({
            "path": relative.to_string_lossy(),
            "text": policy.mask(&text),
            "size": size,
            "modified_ms": modified_ms,
        });
        self.files.insert(
            key.clone(),
            FileEntry {
                version,
                policy: policy.fingerprint,
                body: body.clone(),
                touched: Instant::now(),
            },
        );
        Ok((version, body))
    }

    /// Keep only what `jobs` still watch and anything read in the last
    /// minute, so a read with no watch is not redone on each read.
    fn retain(&mut self, jobs: &[ContentJob]) {
        let now = Instant::now();
        let records: Vec<RecordId> = jobs.iter().filter_map(|j| j.key.record()).collect();
        self.transcripts
            .retain(|id, e| records.contains(id) || now - e.touched < WATCH_IDLE);
        self.files
            .retain(|key, e| jobs.iter().any(|j| &j.key == key) || now - e.touched < WATCH_IDLE);
        self.tickets.retain(|_, e| now - e.touched < WATCH_IDLE);
        self.policies
            .retain(|_, (_, touched)| now - *touched < WATCH_IDLE);
    }
}

fn file_error(e: FileError) -> ContentError {
    match e {
        FileError::TooLarge(size) => ContentError::TooLarge(size),
        other => ContentError::Failed(other.to_string()),
    }
}

/// [`observe`], with a panic in the read answered as `failed` so the
/// worker lives on and a waiting reply is not left hanging. The cache
/// may hold a half-updated entry after a panic, so it starts over.
fn observe_caught(
    host: &dyn ProcessHost,
    transcripts: &dyn TranscriptReader,
    cache: &mut Cache,
    job: &ContentJob,
) -> Observed {
    // `AssertUnwindSafe` says the borrows are fine to use after a
    // panic: the cache is reset below, and the rest is only read.
    let read = std::panic::AssertUnwindSafe(|| observe(host, transcripts, cache, job));
    std::panic::catch_unwind(read).unwrap_or_else(|_| {
        log::warn!(
            "content: the read of {} {} panicked",
            job.key.kind(),
            job.key.id()
        );
        cache.reset();
        Observed {
            key: job.key.clone(),
            policy: 0,
            result: Err(ContentError::Failed("the read panicked".to_owned())),
            reply: job.reply.clone(),
        }
    })
}

/// One job on the worker: the source read whole, versioned by what
/// names its content, masked, cut to the view asked for.
fn observe(
    host: &dyn ProcessHost,
    transcripts: &dyn TranscriptReader,
    cache: &mut Cache,
    job: &ContentJob,
) -> Observed {
    let policy = match cache.job_policy(job) {
        Ok(policy) => policy,
        Err(e) => {
            return Observed {
                key: job.key.clone(),
                policy: 0,
                result: Err(e),
                reply: job.reply.clone(),
            };
        }
    };
    let result = match (&job.source, job.view) {
        (Source::Pane(pane), ContentView::Screen { lines }) => host
            .snapshot(pane, Some(SCREEN_LINES_CAP + SCREEN_SLACK_LINES))
            .map_err(|e| ContentError::Failed(format!("snapshot: {e}")))
            .map(|text| {
                let mut h = DefaultHasher::new();
                text.hash(&mut h);
                let redacted = policy.mask(&text);
                (h.finish(), tail_body(&redacted, lines))
            }),
        (Source::Transcript { handle, root, cwd }, view) => cache
            .conversation(
                transcripts,
                job.key.record().unwrap_or(RecordId(uuid::Uuid::nil())),
                handle,
                root,
                cwd,
                &policy,
            )
            .map(|(source, conversation, links)| {
                let resume_id = handle.provider_id();
                let body = match view {
                    ContentView::Page { from, limit } => {
                        conversation_page(conversation, links, &resume_id, from, limit)
                    }
                    ContentView::Turn { n } => turn_body(conversation, links, &resume_id, n),
                    _ => last_response(conversation, links, &resume_id),
                };
                (source, body)
            }),
        (Source::File { root, relative }, ContentView::Whole) => {
            cache.file(&job.key, root, relative, &policy)
        }
        (Source::Json(value), ContentView::Whole) => cache.ticket_text(&job.key, value, &policy),
        (Source::Log(path), ContentView::Tail { lines }) => {
            scrollback::tail_masked(path, lines, &|s| policy.mask(s), policy.overlap())
                .map_err(|e| ContentError::Failed(format!("log: {e}")))
                .map(|(version, text)| {
                    let mut h = DefaultHasher::new();
                    version.hash(&mut h);
                    let mut body = tail_body(&text, lines);
                    if let ContentKey::RunOutput(_, n) = job.key {
                        body["n"] = serde_json::json!(n);
                    }
                    (h.finish(), body)
                })
        }
        (source, view) => Err(ContentError::Failed(format!(
            "{view:?} is not a view of {source:?}"
        ))),
    };
    Observed {
        key: job.key.clone(),
        policy: policy.fingerprint,
        result,
        reply: job.reply.clone(),
    }
}

/// The last `lines` of a masked text, cut from the top to the byte cap.
fn tail_body(redacted: &str, lines: usize) -> serde_json::Value {
    let kept: Vec<&str> = redacted.trim_end().lines().collect();
    let mut text = kept[kept.len().saturating_sub(lines)..].join("\n");
    let mut truncated = false;
    if text.len() > SCREEN_BYTES_CAP {
        let mut cut = text.len() - SCREEN_BYTES_CAP;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        text = text[cut..].to_owned();
        truncated = true;
    }
    serde_json::json!({ "text": text, "truncated": truncated })
}

fn ms(t: Option<SystemTime>) -> Option<u64> {
    t.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .and_then(|d| u64::try_from(d.as_millis()).ok())
}

/// A page of a conversation on the wire: the metadata, up to `limit`
/// turns or the page's byte cap from the anchor, whichever comes
/// first, and `next`, the turn to continue from, when more follow.
#[must_use]
pub fn conversation_page(
    conversation: &Conversation,
    links: &Links,
    resume_id: &str,
    from: PageFrom,
    limit: usize,
) -> serde_json::Value {
    let total = conversation.turns.len();
    let limit = limit.clamp(1, PAGE_TURNS_CAP);
    let mut bytes = 0;
    let fits = |taken: usize, bytes: usize, size: usize| {
        taken < limit && (taken == 0 || bytes + size <= PAGE_BYTES_CAP)
    };
    let (first, turns, next) = match from {
        PageFrom::Turn(n) => {
            let first = n.max(1);
            let mut turns = Vec::new();
            let mut next = None;
            for turn in conversation.turns.iter().skip(first - 1) {
                let (value, size) = turn_json(turn, links, false);
                if !fits(turns.len(), bytes, size) {
                    next = Some(turn.n);
                    break;
                }
                bytes += size;
                turns.push(value);
            }
            (first, turns, next)
        }
        PageFrom::Last | PageFrom::Before(_) => {
            let end = match from {
                PageFrom::Before(n) => n.saturating_sub(1).min(total),
                _ => total,
            };
            let mut turns = Vec::new();
            for turn in conversation.turns[..end].iter().rev() {
                let (value, size) = turn_json(turn, links, false);
                if !fits(turns.len(), bytes, size) {
                    break;
                }
                bytes += size;
                turns.push(value);
            }
            turns.reverse();
            let first = end + 1 - turns.len();
            let next = (end < total).then_some(end + 1);
            (first, turns, next)
        }
    };
    let usage = conversation.usage;
    serde_json::json!({
        "resume_id": resume_id,
        "title": conversation.title,
        "model": conversation.model,
        "version": conversation.version,
        "branch": conversation.branch,
        "start_ms": ms(conversation.start),
        "end_ms": ms(conversation.end),
        "usage": {
            "input": usage.input,
            "output": usage.output,
            "cache_read": usage.cache_read,
            "cache_create": usage.cache_create,
        },
        "context_tokens": conversation.context_tokens(),
        "total": total,
        "from": first,
        "next": next,
        "turns": turns,
    })
}

/// The last turn's answer on the wire: a projection of the
/// conversation, carrying its version. An empty conversation is turn
/// zero with no text.
#[must_use]
pub fn last_response(
    conversation: &Conversation,
    links: &Links,
    resume_id: &str,
) -> serde_json::Value {
    let last = conversation.turns.last();
    let (text, truncated) = last.map_or((String::new(), false), |t| {
        cap_text(&t.final_text, TURN_BYTES_CAP)
    });
    let links = last.map_or_else(Vec::new, |t| links_in(links, &t.final_text, &text));
    serde_json::json!({
        "resume_id": resume_id,
        "n": last.map_or(0, |t| t.n),
        "at_ms": ms(last.and_then(|t| t.at)),
        "end_ms": ms(last.and_then(|t| t.end)),
        "total": conversation.turns.len(),
        "text": text,
        "truncated": truncated,
        "links": links,
    })
}

/// The links of `original` that still point into `sent`, which is the
/// original whole or its head: a text cut from its front keeps none,
/// since every range would have moved.
fn links_in(links: &Links, original: &str, sent: &str) -> Vec<Link> {
    if !original.starts_with(sent) {
        return Vec::new();
    }
    links
        .get(&file_refs::text_key(original))
        .map(|found| {
            found
                .iter()
                .filter(|l| l.end <= sent.len())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// One turn on the wire and its size. Over the turn cap the tool
/// detail and the agent's notes go first, then the prompt is shortened,
/// then the answer is cut from its front; `truncated` says so.
/// One turn whole on the wire, for a client that opened a turn's
/// activity on a page and wants the detail the page cut.
#[must_use]
pub fn turn_body(
    conversation: &Conversation,
    links: &Links,
    resume_id: &str,
    n: usize,
) -> serde_json::Value {
    let turn = n
        .checked_sub(1)
        .and_then(|i| conversation.turns.get(i))
        .map(|t| turn_json(t, links, true).0);
    serde_json::json!({
        "resume_id": resume_id,
        "total": conversation.turns.len(),
        "turn": turn,
    })
}

/// A turn under its byte cap: whole when it fits, else its tool detail
/// and notes go first, then its prompt is shortened, then its answer is
/// cut from the front. `full` carries each activity's text whole; a
/// page carries the head of each, marked `more`, to stay light.
fn turn_json(turn: &Turn, links: &Links, full: bool) -> (serde_json::Value, usize) {
    let mut lean = false;
    let mut user = turn.user.clone();
    let mut final_text = turn.final_text.clone();
    let mut truncated = false;
    loop {
        let value = turn_value(turn, links, &user, &final_text, lean, truncated, full);
        let size = value.to_string().len();
        if size <= TURN_BYTES_CAP {
            return (value, size);
        }
        truncated = true;
        if !lean {
            lean = true;
        } else if user.len() > 4096 {
            user = cap_text(&user, 4096).0;
        } else if final_text.len() > TURN_BYTES_CAP / 2 {
            final_text = cap_text(&final_text, TURN_BYTES_CAP / 2).0;
        } else {
            return (value, size);
        }
    }
}

fn turn_value(
    turn: &Turn,
    links: &Links,
    user: &str,
    final_text: &str,
    lean: bool,
    truncated: bool,
    full: bool,
) -> serde_json::Value {
    let head = |text: &str| {
        if full {
            (text.to_owned(), false)
        } else {
            head_text(text, ACTIVITY_BYTES_CAP)
        }
    };
    let activity: Vec<serde_json::Value> = turn
        .activity
        .iter()
        .map(|a| {
            let kind = match a.kind {
                ActivityKind::Tool => "tool",
                ActivityKind::Text => "text",
                ActivityKind::System => "system",
            };
            let mut v = serde_json::json!({
                "kind": kind,
                "line": a.line,
                "at_ms": ms(a.at),
                "error": a.error,
            });
            if !lean {
                let mut more = false;
                if let Some(d) = &a.detail {
                    let (input, cut_input) = head(&d.input);
                    let (result, cut_result) = head(&d.result);
                    more |= cut_input || cut_result;
                    v["detail"] = serde_json::json!({
                        "name": d.name,
                        "input": input,
                        "result": result,
                        "path": d.path.as_ref().map(|p| p.display().to_string()),
                    });
                }
                if let Some(t) = &a.text {
                    let (text, cut) = head(t);
                    more |= cut;
                    v["text"] = serde_json::json!(text);
                }
                if more {
                    v["more"] = serde_json::json!(true);
                }
                // The links are into the text the row draws: the
                // message, or its one line when that is all there is.
                if a.kind == ActivityKind::Text {
                    let original = file_refs::message_text(a);
                    let sent = v["text"].as_str().unwrap_or(&a.line).to_owned();
                    v["links"] = serde_json::json!(links_in(links, original, &sent));
                }
            }
            v
        })
        .collect();
    serde_json::json!({
        "n": turn.n,
        "at_ms": ms(turn.at),
        "end_ms": ms(turn.end),
        "user": user,
        "final_text": final_text,
        "links": links_in(links, &turn.final_text, final_text),
        "activity": activity,
        "assistant_msgs": turn.assistant_msgs,
        "tools": turn.tools,
        "thinking": turn.thinking,
        "errors": turn.errors,
        "permission_mode": turn.permission_mode,
        "truncated": truncated,
    })
}

/// The first `cap` bytes of `text` on a character boundary, and
/// whether anything was cut.
fn head_text(text: &str, cap: usize) -> (String, bool) {
    if text.len() <= cap {
        return (text.to_owned(), false);
    }
    let mut cut = cap;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    (text[..cut].to_owned(), true)
}

/// The last `cap` bytes of `text` on a character boundary, and
/// whether anything was cut.
fn cap_text(text: &str, cap: usize) -> (String, bool) {
    if text.len() <= cap {
        return (text.to_owned(), false);
    }
    let mut cut = text.len() - cap;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    (text[cut..].to_owned(), true)
}

/// `text` with every occurrence of each secret's value replaced by
/// `<NAME>`. Longer values go first, so a value that holds another is
/// replaced whole rather than leaving a fragment around the shorter one.
/// A pane's capture breaks a line longer than the pane where it wraps,
/// so a value is also found with line breaks inside it.
/// The capture also trims each row's trailing spaces, so a space in the
/// value that fell at a wrap may be gone.
#[must_use]
pub fn redact(text: &str, secrets: &[(String, String)]) -> String {
    let mut by_length: Vec<&(String, String)> =
        secrets.iter().filter(|(_, v)| !v.is_empty()).collect();
    by_length.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    let mut out = text.to_owned();
    for (name, value) in by_length {
        let placeholder = format!("<{name}>");
        let mut from = 0;
        while let Some((start, end)) = find_across_breaks(&out, value, from) {
            out.replace_range(start..end, &placeholder);
            from = start + placeholder.len();
        }
    }
    out
}

/// The byte range of the first occurrence of `value` in `text` at or
/// after `from`, where `text` may break a line anywhere inside it and
/// some of a run of spaces in `value` at such a break may be missing:
/// the row before the break lost its trailing ones, and the row after
/// holds the rest. Byte matching stays on character boundaries: a
/// value's first byte is never a UTF-8 continuation byte, and a line
/// break never sits inside one.
fn find_across_breaks(text: &str, value: &str, from: usize) -> Option<(usize, usize)> {
    let hay = text.as_bytes();
    let needle = value.as_bytes();
    let is_break = |b: u8| b == b'\n' || b == b'\r';
    let spaces = |bytes: &[u8], at: usize| {
        bytes
            .get(at..)
            .map_or(0, |rest| rest.iter().take_while(|&&b| b == b' ').count())
    };
    (from..hay.len()).find_map(|start| {
        let mut i = start;
        let mut k = 0;
        while let Some(&want) = needle.get(k) {
            let at_break = hay.get(i).copied().is_some_and(is_break);
            if k > 0 && !is_break(want) && at_break {
                while hay.get(i).copied().is_some_and(is_break) {
                    i += 1;
                }
                if want == b' ' {
                    let kept = spaces(hay, i);
                    let run = spaces(needle, k);
                    if kept > run {
                        return None;
                    }
                    i += kept;
                    k += run;
                    continue;
                }
            }
            if hay.get(i) != Some(&want) {
                return None;
            }
            i += 1;
            k += 1;
        }
        Some((start, i))
    })
}

/// One parsed transcript and where it came from.
struct Entry {
    /// The handle the transcript was read through. A record rebound to
    /// another handle (`/clear`, a discard) needs a fresh read. `None`
    /// is a conversation with no file behind it (tests, the script),
    /// which no handle makes stale.
    handle: Option<ResumeHandle>,
    /// The file's modification time at the read; `None` when the reader
    /// has no file time, which then matches `None` on the next check.
    modified: Option<SystemTime>,
    conversation: Conversation,
}

/// Parsed transcripts per agent session, each with the handle and file
/// time it was read at, and why a session has none.
#[derive(Default)]
pub struct ConversationCache {
    entries: HashMap<RecordId, Entry>,
    errors: HashMap<RecordId, String>,
}

impl ConversationCache {
    /// The session's conversation, if one was read.
    #[must_use]
    pub fn get(&self, id: RecordId) -> Option<&Conversation> {
        self.entries.get(&id).map(|e| &e.conversation)
    }

    /// The conversation for editing in place: tests shape a turn after
    /// seeding it.
    pub fn get_mut(&mut self, id: RecordId) -> Option<&mut Conversation> {
        self.entries.get_mut(&id).map(|e| &mut e.conversation)
    }

    /// Why the session has no conversation (unsupported agent, no file).
    #[must_use]
    pub fn error(&self, id: RecordId) -> Option<&str> {
        self.errors.get(&id).map(String::as_str)
    }

    #[must_use]
    pub fn contains(&self, id: RecordId) -> bool {
        self.entries.contains_key(&id)
    }

    /// Whether what is held for `id` was read through `handle` at
    /// `modified`, so no read is needed.
    #[must_use]
    pub fn is_current(
        &self,
        id: RecordId,
        handle: &ResumeHandle,
        modified: Option<SystemTime>,
    ) -> bool {
        self.entries.get(&id).is_some_and(|e| {
            e.handle.as_ref().is_none_or(|h| h == handle) && e.modified == modified
        })
    }

    /// Keep a conversation read through `handle` at `modified`; clears
    /// any error for the session.
    pub fn insert(
        &mut self,
        id: RecordId,
        handle: ResumeHandle,
        modified: Option<SystemTime>,
        conversation: Conversation,
    ) {
        self.errors.remove(&id);
        self.entries.insert(
            id,
            Entry {
                handle: Some(handle),
                modified,
                conversation,
            },
        );
    }

    /// Keep a conversation with no file behind it, for tests and the
    /// script: it stays until removed.
    pub fn seed(&mut self, id: RecordId, conversation: Conversation) {
        self.errors.remove(&id);
        self.entries.insert(
            id,
            Entry {
                handle: None,
                modified: None,
                conversation,
            },
        );
    }

    /// Record that the session's transcript could not be read; drops
    /// whatever was held.
    pub fn fail(&mut self, id: RecordId, reason: String) {
        self.entries.remove(&id);
        self.errors.insert(id, reason);
    }

    /// Forget the session's conversation so the next refresh reads it.
    pub fn remove(&mut self, id: RecordId) {
        self.entries.remove(&id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::transcript::Activity;

    fn turns(n: usize) -> Conversation {
        let mut c = Conversation::default();
        for i in 1..=n {
            c.turns.push(Turn {
                n: i,
                user: format!("prompt {i}"),
                final_text: format!("answer {i}"),
                ..Turn::default()
            });
        }
        c
    }

    #[test]
    fn prose_paths_under_the_root_become_links_and_a_cut_text_keeps_only_those_it_still_holds() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("sub");
        std::fs::create_dir_all(cwd.join("deep")).unwrap();
        std::fs::write(root.path().join("plan.md"), "").unwrap();
        std::fs::write(cwd.join("deep").join("a.rs"), "").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("notes.md"), "").unwrap();
        let mut c = turns(1);
        c.turns[0].final_text = format!(
            "See plan.md and `deep/a.rs:12`, not {} or gone.md.",
            outside.path().join("notes.md").display()
        );
        c.turns[0].activity.push(Activity {
            kind: ActivityKind::Text,
            line: "Edited plan.md".into(),
            at: None,
            error: false,
            detail: None,
            text: Some("Edited plan.md first".into()),
        });
        let links = link_map(&c, root.path(), &cwd, &Links::new());
        let page = conversation_page(&c, &links, "r", PageFrom::Turn(1), 10);
        let turn = &page["turns"][0];
        assert_eq!(
            turn["links"],
            serde_json::json!([
                {"start": 4, "end": 11, "path": "plan.md", "line": null},
                {"start": 16, "end": 30, "path": "sub/deep/a.rs", "line": 12},
            ]),
            "{turn}"
        );
        assert_eq!(turn["activity"][0]["links"][0]["path"], "plan.md");
        let last = last_response(&c, &links, "r");
        assert_eq!(last["links"][1]["line"], 12);

        // A message cut to its head keeps the links in the head.
        c.turns[0].activity[0].text = Some(format!("Edited plan.md, {}", "x".repeat(2000)));
        let links = link_map(&c, root.path(), &cwd, &links);
        let page = conversation_page(&c, &links, "r", PageFrom::Turn(1), 10);
        let row = &page["turns"][0]["activity"][0];
        assert_eq!(row["more"], true);
        assert_eq!(row["links"][0]["path"], "plan.md", "{row}");
        // One cut from its front keeps none.
        c.turns[0].final_text = format!("plan.md {}", "y".repeat(TURN_BYTES_CAP));
        let links = link_map(&c, root.path(), &cwd, &links);
        let page = conversation_page(&c, &links, "r", PageFrom::Turn(1), 10);
        assert_eq!(page["turns"][0]["truncated"], true);
        assert_eq!(page["turns"][0]["links"], serde_json::json!([]));
    }

    #[test]
    fn a_page_stops_at_its_limit_and_says_where_to_continue() {
        let c = turns(5);
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Turn(1), 2);
        assert_eq!(page["total"], 5);
        assert_eq!(page["from"], 1);
        assert_eq!(page["next"], 3);
        assert_eq!(page["turns"].as_array().unwrap().len(), 2);
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Turn(3), 10);
        assert_eq!(page["next"], serde_json::Value::Null);
        assert_eq!(page["turns"][0]["n"], 3);
        assert_eq!(page["turns"].as_array().unwrap().len(), 3);
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Last, 2);
        assert_eq!(page["from"], 4, "the last two");
        assert_eq!(page["next"], serde_json::Value::Null);
        assert_eq!(page["turns"][1]["final_text"], "answer 5");
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Before(4), 2);
        assert_eq!(page["from"], 2, "the two before turn 4");
        assert_eq!(page["next"], 4);
        assert_eq!(page["turns"][1]["n"], 3);
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Before(1), 2);
        assert!(page["turns"].as_array().unwrap().is_empty());
        assert_eq!(page["from"], 1);
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Before(99), 2);
        assert_eq!(page["from"], 4, "past the end: the tail");
        assert_eq!(page["next"], serde_json::Value::Null);
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Turn(9), 2);
        assert!(page["turns"].as_array().unwrap().is_empty(), "past the end");
        assert_eq!(page["next"], serde_json::Value::Null);
    }

    #[test]
    fn an_oversized_turn_is_cut_and_marked_with_its_answer_kept() {
        let mut c = turns(1);
        c.turns[0]
            .activity
            .push(crate::ports::transcript::Activity {
                kind: ActivityKind::Tool,
                line: "Read x".into(),
                at: None,
                error: false,
                detail: Some(crate::ports::transcript::ToolDetail {
                    name: "Read".into(),
                    input: "x".repeat(TURN_BYTES_CAP),
                    result: String::new(),
                    path: None,
                }),
                text: None,
            });
        // On a page the detail is the head of itself, so the turn fits.
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Turn(1), 1);
        let turn = &page["turns"][0];
        assert_eq!(turn["truncated"], false);
        assert_eq!(turn["activity"][0]["more"], true);
        // Read whole, the detail is over the turn's cap and goes.
        let whole = turn_body(&c, &Links::new(), "r", 1);
        let turn = &whole["turn"];
        assert_eq!(turn["truncated"], true);
        assert_eq!(turn["final_text"], "answer 1");
        assert!(
            turn["activity"][0].get("detail").is_none(),
            "the detail went"
        );
        assert_eq!(turn["activity"][0]["line"], "Read x");
    }

    #[test]
    fn a_page_stops_before_its_byte_cap_but_always_carries_one_turn() {
        let mut c = turns(4);
        for t in &mut c.turns {
            t.final_text = "y".repeat(200 * 1024);
        }
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Turn(1), 10);
        assert_eq!(page["turns"].as_array().unwrap().len(), 3);
        assert_eq!(page["next"], 4);
        // A turn over its own cap is cut first, and the cut size counts.
        c.turns[0].final_text = "y".repeat(PAGE_BYTES_CAP);
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Turn(1), 10);
        assert_eq!(page["turns"][0]["truncated"], true);
        assert_eq!(page["turns"].as_array().unwrap().len(), 3);
        assert_eq!(page["next"], 4);
    }

    #[test]
    fn a_page_anchored_at_its_end_fills_backward_and_always_reaches_it() {
        let mut c = turns(8);
        for t in &mut c.turns {
            t.final_text = "y".repeat(200 * 1024);
        }
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Last, 20);
        let ns: Vec<_> = page["turns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["n"].as_u64().unwrap())
            .collect();
        assert_eq!(ns, [6, 7, 8], "the last turns that fit, ending at the end");
        assert_eq!(page["from"], 6);
        assert_eq!(page["next"], serde_json::Value::Null);
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Before(6), 20);
        assert_eq!(page["from"], 3);
        assert_eq!(page["next"], 6);
        assert_eq!(page["turns"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_page_carries_the_head_of_each_tool_detail_and_a_turn_read_alone_is_whole() {
        let mut c = turns(2);
        c.turns[1]
            .activity
            .push(crate::ports::transcript::Activity {
                kind: ActivityKind::Tool,
                line: "Read x".into(),
                at: None,
                error: false,
                detail: Some(crate::ports::transcript::ToolDetail {
                    name: "Read".into(),
                    input: "i".repeat(ACTIVITY_BYTES_CAP + 1),
                    result: "héllo".into(),
                    path: None,
                }),
                text: None,
            });
        let page = conversation_page(&c, &Links::new(), "r", PageFrom::Turn(1), 10);
        let activity = &page["turns"][1]["activity"][0];
        assert_eq!(activity["more"], true);
        assert_eq!(
            activity["detail"]["input"].as_str().unwrap().len(),
            ACTIVITY_BYTES_CAP
        );
        assert_eq!(
            activity["detail"]["result"], "héllo",
            "short: whole, no mark"
        );
        assert!(page["turns"][0]["activity"].as_array().unwrap().is_empty());

        let whole = turn_body(&c, &Links::new(), "r", 2);
        assert_eq!(whole["total"], 2);
        let activity = &whole["turn"]["activity"][0];
        assert!(activity.get("more").is_none());
        assert_eq!(
            activity["detail"]["input"].as_str().unwrap().len(),
            ACTIVITY_BYTES_CAP + 1
        );
        assert_eq!(
            turn_body(&c, &Links::new(), "r", 3)["turn"],
            serde_json::Value::Null
        );
        assert_eq!(
            turn_body(&c, &Links::new(), "r", 0)["turn"],
            serde_json::Value::Null
        );
        assert_eq!(
            head_text("héllo", 2).0,
            "h",
            "a cut lands on a character boundary"
        );
    }

    #[test]
    fn the_last_response_is_the_last_turns_answer() {
        assert_eq!(
            last_response(&Conversation::default(), &Links::new(), "r")["n"],
            0
        );
        let last = last_response(&turns(3), &Links::new(), "r");
        assert_eq!(last["n"], 3);
        assert_eq!(last["text"], "answer 3");
        assert_eq!(last["total"], 3);
    }

    #[test]
    fn a_run_output_key_is_the_record_and_the_run_number() {
        let id = RecordId::new();
        let key = ContentKey::RunOutput(id, 4);
        assert_eq!(ContentKey::parse("run_output", &key.id()), Some(key));
        assert_eq!(ContentKey::parse("run_output", &id.0.to_string()), None);
        assert_eq!(
            ContentKey::parse("conversation", &id.0.to_string()),
            Some(ContentKey::Conversation(id))
        );
    }

    fn handle(s: &str) -> ResumeHandle {
        ResumeHandle::Codex {
            rollout_id: s.into(),
            transcript: None,
        }
    }

    #[test]
    fn a_read_is_current_for_its_handle_and_time() {
        let id = RecordId::new();
        let mut cache = ConversationCache::default();
        let at = Some(SystemTime::UNIX_EPOCH);
        cache.insert(id, handle("a"), at, Conversation::default());
        assert!(cache.is_current(id, &handle("a"), at));
        assert!(!cache.is_current(id, &handle("b"), at), "rebound");
        assert!(!cache.is_current(id, &handle("a"), None), "file changed");
        assert!(!cache.is_current(RecordId::new(), &handle("a"), at));
    }

    #[test]
    fn a_seeded_conversation_is_current_for_any_handle() {
        let id = RecordId::new();
        let mut cache = ConversationCache::default();
        cache.seed(id, Conversation::default());
        assert!(cache.is_current(id, &handle("a"), None));
        assert!(!cache.is_current(id, &handle("a"), Some(SystemTime::UNIX_EPOCH)));
    }

    #[test]
    fn a_failure_replaces_the_conversation_and_a_read_clears_it() {
        let id = RecordId::new();
        let mut cache = ConversationCache::default();
        cache.seed(id, Conversation::default());
        cache.fail(id, "no file".into());
        assert!(!cache.contains(id));
        assert_eq!(cache.error(id), Some("no file"));
        cache.insert(id, handle("a"), None, Conversation::default());
        assert!(cache.contains(id));
        assert_eq!(cache.error(id), None);
    }

    fn set_spec() -> PolicySpec {
        PolicySpec {
            epoch: 0,
            project: ProjectId(uuid::Uuid::nil()),
            root: PathBuf::from("/nonexistent"),
            global: Vec::new(),
            project_env: ProjectEnv::default(),
            sets: vec![EnvSet {
                name: "deploy".into(),
                vars: vec![EnvVar {
                    name: "KEY".into(),
                    value: String::new(),
                    secret: true,
                }],
                aws: None,
            }],
            project_sets: Vec::new(),
            record_sets: vec!["deploy".into()],
        }
    }

    #[test]
    fn a_store_that_fails_on_a_set_secret_fails_the_policy() {
        let secrets = crate::adapters::fakes::FakeSecrets::default();
        secrets.state().insert(
            crate::core::SecretScope::Set("deploy".into()).account("KEY"),
            "v-1".into(),
        );
        let policy = resolve_policy(&set_spec(), &secrets).unwrap();
        assert_eq!(policy.secrets, [("KEY".to_owned(), "v-1".to_owned())]);
        secrets.set_failing(true);
        assert!(resolve_policy(&set_spec(), &secrets).is_err());
    }

    #[test]
    fn json_is_masked_in_its_strings_before_it_is_escaped() {
        let policy = Policy::new(vec![("KEY".into(), "a\"b\\c".into())]);
        let mut value = serde_json::json!({
            "a\"b\\c": ["x a\"b\\c y", 3, {"text": "a\"b\\c"}],
        });
        mask_json(&mut value, &policy);
        assert_eq!(
            value,
            serde_json::json!({ "a\"b\\c": ["x <KEY> y", 3, {"text": "<KEY>"}] }),
            "values masked, keys left"
        );
    }

    /// A worker over a fake host, with what it observes on `rx`.
    fn worker(
        host: &crate::adapters::fakes::FakeHost,
    ) -> (ContentWorker, mpsc::Receiver<Observed>) {
        let (tx, rx) = mpsc::channel();
        let worker = ContentWorker::spawn(
            Arc::new(host.clone()),
            Arc::new(crate::adapters::fakes::FakeTranscripts::default()),
            Arc::new(crate::adapters::fakes::FakeSecrets::default()),
            tx,
            Arc::new(|| {}),
        );
        (worker, rx)
    }

    fn screen_job(pane: &str) -> ContentJob {
        ContentJob {
            key: ContentKey::Screen(RecordId(uuid::Uuid::nil())),
            source: Source::Pane(HostId(pane.into())),
            view: ContentView::Screen { lines: 10 },
            policies: vec![PolicySpec {
                sets: Vec::new(),
                record_sets: Vec::new(),
                ..set_spec()
            }],
            reply: None,
        }
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn a_read_that_panics_answers_failed_and_the_worker_reads_on() {
        let host = crate::adapters::fakes::FakeHost::default();
        host.state()
            .snapshots
            .insert(HostId("p".into()), "hello".into());
        host.state().panic_snapshot = true;
        let (worker, rx) = worker(&host);
        assert!(worker.submit(screen_job("p")));
        let out = rx
            .recv_timeout(WAIT)
            .expect("an answer for the panicked read");
        assert_eq!(
            out.result.unwrap_err(),
            ContentError::Failed("the read panicked".into())
        );
        assert!(worker.submit(screen_job("p")));
        let out = rx.recv_timeout(WAIT).expect("the worker lives on");
        assert!(out.result.is_ok());
    }

    #[test]
    fn a_poisoned_poll_lock_still_polls_and_takes_updates() {
        let host = crate::adapters::fakes::FakeHost::default();
        let (worker, rx) = worker(&host);
        worker.set_watch(vec![screen_job("p")]);
        let poll = Arc::clone(&worker.poll);
        let _ = std::thread::spawn(move || {
            let _held = poll.lock().unwrap();
            panic!("poison the poll lock");
        })
        .join();
        let watch = Arc::clone(&worker.watch);
        let _ = std::thread::spawn(move || {
            let _held = watch.lock().unwrap();
            panic!("poison the watch lock");
        })
        .join();
        assert!(worker.watch.is_poisoned());
        // `set_poll` takes its update through the poisoned poll lock; no
        // `set_watch` follows, so only the worker's own poll read can
        // clear the watch poison and answer with the job's data.
        worker.set_poll(Duration::from_millis(10));
        assert_eq!(*worker.poll.lock().unwrap(), Duration::from_millis(10));
        let out = rx.recv_timeout(WAIT).expect("the poll reads the watch");
        assert_eq!(out.key, ContentKey::Screen(RecordId(uuid::Uuid::nil())));
        assert!(!worker.watch.is_poisoned());
    }
}
