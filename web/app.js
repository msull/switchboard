// Switchboard's web page: pair a browser from the pairing window's QR,
// list the sessions (live ones by default, waiting first, narrowed by
// state, kind, project and a name search), read one (a Claude Code
// session as its conversation, anything else as its screen), and
// reply.
// It speaks the remote port's /api/v1 as a native client does, with
// two differences: the token is a cookie this script never sees, and
// changes arrive by polling, the events socket only hastening them.
// Text from the host is set with textContent only, never as markup.
'use strict';

// Every route the page calls, method and path. A host test reads this
// table and checks that each one is served.
const API = {
  info: 'GET /api/v1/info',
  pair: 'POST /api/v1/pair',
  me: 'GET /api/v1/me',
  projects: 'GET /api/v1/projects',
  sessions: 'GET /api/v1/projects/{id}/sessions',
  waiting: 'GET /api/v1/waiting',
  conversation: 'GET /api/v1/sessions/{id}/conversation',
  screen: 'GET /api/v1/sessions/{id}/screen',
  send: 'POST /api/v1/sessions/{id}/send',
  events: 'GET /api/v1/events',
};

const STORE_KEY = 'switchboard-host';
const POLL_FAST_MS = 3000;
const POLL_SLOW_MS = 20000;
const SOCKET_RETRY_MS = 5000;
const SOCKET_RETRY_MAX_MS = 60000;
// Turns per conversation page: the host's default, small enough for a
// phone, where a page at the host's cap could be 640 KiB.
const TURNS_PER_PAGE = 20;

// --- pure parts, tested with node --test

/** The invite in a URL fragment, `#code=…&key=…&id=…`, or null. */
function parseFragment(hash) {
  const text = (hash || '').replace(/^#/, '');
  const fields = {};
  for (const pair of text.split('&')) {
    const at = pair.indexOf('=');
    if (at <= 0) continue;
    try {
      fields[pair.slice(0, at)] = decodeURIComponent(pair.slice(at + 1));
    } catch (_) {
      return null;
    }
  }
  if (!fields.code || !fields.key || !fields.id) return null;
  return { code: fields.code, key: fields.key.toLowerCase(), id: fields.id.toLowerCase() };
}

/** "Safari on iPhone", from the user agent; the host shows it. */
function deviceName(ua, touchPoints) {
  ua = ua || '';
  let browser = 'Safari';
  if (/Edg\//.test(ua)) browser = 'Edge';
  else if (/Chrome\/|CriOS\//.test(ua)) browser = 'Chrome';
  else if (/Firefox\/|FxiOS\//.test(ua)) browser = 'Firefox';
  else if (!/Safari\//.test(ua)) return 'Browser';
  let device = null;
  if (/iPhone/.test(ua)) device = 'iPhone';
  else if (/iPad/.test(ua) || (/Macintosh/.test(ua) && touchPoints > 1)) device = 'iPad';
  else if (/Android/.test(ua)) device = 'Android';
  else if (/Macintosh|Mac OS X/.test(ua)) device = 'Mac';
  else if (/Windows/.test(ua)) device = 'Windows';
  else if (/Linux/.test(ua)) device = 'Linux';
  return device ? `${browser} on ${device}` : 'Browser';
}

/**
 * What a 401 means, told from `/info`, which answers anyone: the host
 * this page paired with, under the same key, no longer knows the device
 * ("removed"); a different key or id ("changed"); no answer
 * ("unreachable"); or nothing was ever paired here ("unpaired").
 */
function inferUnauthorized(stored, info) {
  if (!stored) return 'unpaired';
  if (!info) return 'unreachable';
  const same = String(info.server_id).toLowerCase() === stored.server_id
    && String(info.spki_sha256).toLowerCase() === stored.spki_sha256;
  return same ? 'removed' : 'changed';
}

/**
 * Whether a reply at `next` may replace what is held at `held`: a
 * restarted host (a new epoch) always may; otherwise only a revision at
 * or past the held one, so a late reply never undoes a newer one.
 */
function isCurrent(held, next) {
  if (!held) return true;
  if (held.epoch !== next.epoch) return true;
  return next.revision >= held.revision;
}

// The list's choices: which sessions show and in what order.
const VIEW_KEY = 'switchboard-list';
const STATES = ['live', 'waiting', 'ended', 'all'];
const KINDS = ['all', 'claude', 'codex', 'shell', 'command', 'service'];
const SORTS = ['waiting', 'recent', 'project', 'name'];
const DEFAULT_VIEW = Object.freeze({ state: 'live', kind: 'all', projects: Object.freeze([]), sort: 'waiting' });

/**
 * The list's choices from what storage held, which may be anything:
 * each field that is missing, mistyped or unknown takes its default.
 */
function readView(raw) {
  const v = raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {};
  return {
    state: STATES.includes(v.state) ? v.state : DEFAULT_VIEW.state,
    kind: KINDS.includes(v.kind) ? v.kind : DEFAULT_VIEW.kind,
    projects: Array.isArray(v.projects) ? v.projects.filter((p) => typeof p === 'string') : [...DEFAULT_VIEW.projects],
    sort: SORTS.includes(v.sort) ? v.sort : DEFAULT_VIEW.sort,
  };
}

/**
 * Whether the pane runs. Liveness is the string `running`, or `missing`,
 * or an object for an exited pane; only the first is live, whatever the
 * card says.
 */
function isLive(s) {
  return s.liveness === 'running';
}

/**
 * When the session last did anything, in milliseconds: the latest of
 * the pane printing, the record being seen, and the agent stopping. An
 * ended session has only the last two.
 */
function recencyMs(s) {
  return Math.max(s.last_activity_ms || 0, s.last_seen_ms || 0, s.last_stop_at_ms || 0);
}

function passesState(s, state) {
  if (state === 'live') return isLive(s);
  if (state === 'waiting') return !!s.waiting;
  if (state === 'ended') return !isLive(s);
  return true;
}

const SORTERS = {
  // `arrangeSessions` coerces `waiting` to a boolean, so subtracting is safe.
  waiting: (a, b) => (b.session.waiting - a.session.waiting) || (recencyMs(b.session) - recencyMs(a.session)),
  recent: (a, b) => recencyMs(b.session) - recencyMs(a.session),
  project: (a, b) => a.project.name.localeCompare(b.project.name) || (recencyMs(b.session) - recencyMs(a.session)),
  name: (a, b) => String(a.session.name).localeCompare(String(b.session.name)),
};

/**
 * The sessions to show under `view` and the `search` text, one flat
 * list with each row's project: state, kind and projects narrow it (a
 * selection naming no project that exists means all), then a
 * case-insensitive match on the name. The host's order breaks every tie,
 * so rows that compare equal never reshuffle. `hiddenWaiting` counts
 * the waiting sessions any of that hid.
 */
function arrangeSessions(projects, sessionsByProject, view, search) {
  const known = new Set(projects.map((p) => p.id));
  const picked = new Set(view.projects.filter((id) => known.has(id)));
  const needle = (search || '').trim().toLowerCase();
  const all = [];
  for (const project of projects) {
    for (const s of sessionsByProject[project.id] || []) {
      all.push({ project, session: { ...s, waiting: !!s.waiting } });
    }
  }
  const rows = all.filter(({ project, session }) => passesState(session, view.state)
    && (view.kind === 'all' || session.kind === view.kind)
    && (picked.size === 0 || picked.has(project.id))
    && (!needle || String(session.name).toLowerCase().includes(needle)));
  const shown = new Set(rows);
  const hiddenWaiting = all.filter((r) => r.session.waiting && !shown.has(r)).length;
  const sorter = SORTERS[view.sort] || SORTERS.waiting;
  const sorted = rows
    .map((row, i) => ({ row, i }))
    .sort((a, b) => sorter(a.row, b.row) || (a.i - b.i))
    .map((x) => x.row);
  return { rows: sorted, total: all.length, shown: sorted.length, hiddenWaiting };
}

/**
 * What an empty list says: nothing at all, nothing live under the
 * default view (which is also what the host's first moments look like,
 * before it has listed its panes), or nothing matching.
 */
function emptyText(result, view, search) {
  if (result.total === 0) return 'No sessions.';
  const isDefault = view.state === 'live' && view.kind === 'all'
    && view.projects.length === 0 && !(search || '').trim();
  if (isDefault) return `No live sessions; ${result.total} not running.`;
  return 'Nothing matches these filters.';
}

/**
 * How many of the band's choices differ from the defaults. A stored
 * project that no longer exists narrows nothing, so it does not count.
 */
function changedCount(view, projects) {
  const known = new Set(projects.map((p) => p.id));
  return ['state', 'kind', 'sort'].filter((k) => view[k] !== DEFAULT_VIEW[k]).length
    + (view.projects.some((id) => known.has(id)) ? 1 : 0);
}

/** "just now", "5 min ago", "3 h ago", "2 d ago". */
function sinceText(ms, now) {
  if (ms == null) return '';
  const secs = Math.max(0, Math.floor((now - ms) / 1000));
  if (secs < 60) return 'just now';
  if (secs < 3600) return `${Math.floor(secs / 60)} min ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} h ago`;
  return `${Math.floor(secs / 86400)} d ago`;
}

/** A route from the table with its `{id}` filled. */
function route(entry, id) {
  const [method, path] = entry.split(' ');
  return { method, path: path.replace('{id}', encodeURIComponent(id || '')) };
}

/**
 * The query for a conversation page: the last turns, or those ending
 * before turn `before`.
 */
function conversationQuery(before) {
  const anchor = before ? `before=${before}` : 'from=last';
  return `?${anchor}&limit=${TURNS_PER_PAGE}`;
}

/**
 * The turns held after a page arrives, ordered by number `n`. A page's
 * turn replaces a held one of the same number; held turns past the
 * conversation's `total` are gone; a page that starts past the turn
 * after the last held one replaces them all, since keeping them would
 * show a stale middle as if nothing were missing, and so does a page
 * that ends short of the turn before the first held one.
 */
function mergeTurns(held, page) {
  const turns = page.turns || [];
  let kept = (held || []).filter((t) => page.total == null || t.n <= page.total);
  if (turns.length === 0) return kept;
  const highest = kept.length ? kept[kept.length - 1].n : 0;
  if (turns[0].n > highest + 1) kept = [];
  // The same on the low side: a page ending short of the first held
  // turn would leave a gap below it.
  if (kept.length && turns[turns.length - 1].n < kept[0].n - 1) kept = [];
  const byNumber = new Map(kept.map((t) => [t.n, t]));
  for (const t of turns) byNumber.set(t.n, t);
  return [...byNumber.values()].sort((a, b) => a.n - b.n);
}

/**
 * What a session's list entry is compared by, to tell whether the open
 * conversation needs reading again: `key` is the whole entry but
 * `quiet_secs`, which ages with the clock (the host's revision leaves it
 * out too); `card` and `stop` mark a turn's boundary.
 */
function entryMark(s) {
  return {
    key: JSON.stringify({ ...s, quiet_secs: undefined }),
    card: s.card,
    stop: s.last_stop_at_ms,
  };
}

/**
 * Whether the entry crossed a turn's boundary between two marks: its
 * card or its last stop changed. False when either mark is unknown.
 */
function turnBoundary(prev, next) {
  if (!prev || !next) return false;
  return prev.card !== next.card || prev.stop !== next.stop;
}

/**
 * The turn to ask for the turns before, or null when nothing is held or
 * the first turn already is.
 */
function earlierBefore(turns) {
  if (!turns || turns.length === 0) return null;
  const lowest = turns[0].n;
  return lowest > 1 ? lowest : null;
}

/**
 * What the reply box holds once a send of `sent` (already trimmed) has
 * succeeded: what was typed after it, nothing if there is none, or the
 * whole box if it no longer starts with the sent text.
 */
function afterSend(box, sent) {
  const rest = box.trimStart();
  if (!rest.startsWith(sent)) return box;
  const typed = rest.slice(sent.length);
  return typed.trim() === '' ? '' : typed;
}

if (typeof module !== 'undefined' && module.exports) {
  module.exports = {
    API,
    parseFragment,
    deviceName,
    inferUnauthorized,
    isCurrent,
    DEFAULT_VIEW,
    readView,
    isLive,
    recencyMs,
    arrangeSessions,
    emptyText,
    changedCount,
    sinceText,
    route,
    conversationQuery,
    mergeTurns,
    entryMark,
    turnBoundary,
    earlierBefore,
    afterSend,
  };
}

// --- the page

if (typeof document !== 'undefined') {
  const $ = (id) => document.getElementById(id);
  const state = {
    me: null,
    listMark: null,
    open: null,
    sessions: {},
    // The last list loaded, which the list's controls arrange without
    // asking the host again.
    loaded: { projects: [], sessionsByProject: {} },
    view: readView(null),
    search: '',
    projectChoices: null,
    held: {},
    // The open conversation's turns, kept in memory only.
    turns: [],
    resumeId: null,
    // The `entryMark` of the entry the drawn conversation was read
    // against, null when unknown, which makes the next refresh read.
    readMark: null,
    // The timer of a pending confirming re-read after a turn's boundary.
    confirm: null,
    scrollToEnd: false,
    pending: null,
    chain: Promise.resolve(),
    socket: null,
    socketRetry: SOCKET_RETRY_MS,
    pollMs: POLL_FAST_MS,
    timer: null,
  };

  class HttpError extends Error {
    constructor(status, code, message) {
      super(message || code);
      this.status = status;
      this.code = code;
    }
  }

  async function call(entry, id, body, query) {
    const { method, path } = route(entry, id);
    const init = { method, credentials: 'same-origin', cache: 'no-store', headers: {} };
    if (body !== undefined) {
      init.headers['Content-Type'] = 'application/json';
      init.body = JSON.stringify(body);
    }
    const response = await fetch(path + (query || ''), init);
    let json = null;
    try { json = await response.json(); } catch (_) { json = null; }
    if (!response.ok) {
      const error = (json && json.error) || {};
      throw new HttpError(response.status, error.code || 'failed', error.message);
    }
    return json;
  }

  function stored() {
    try { return JSON.parse(localStorage.getItem(STORE_KEY)); } catch (_) { return null; }
  }

  /**
   * The list's choices this browser kept. Storage may be refused or
   * cleared, so the defaults are always a whole answer.
   */
  function storedView() {
    try { return readView(JSON.parse(localStorage.getItem(VIEW_KEY))); } catch (_) { return readView(null); }
  }

  function saveView() {
    try { localStorage.setItem(VIEW_KEY, JSON.stringify(state.view)); } catch (_) { /* kept for this visit only */ }
  }

  function show(section) {
    for (const id of ['notice', 'list', 'session']) $(id).hidden = id !== section;
    $('header').hidden = section === 'notice' || !state.me;
  }

  function notice(title, text) {
    $('notice-title').textContent = title;
    $('notice-text').textContent = text;
    show('notice');
  }

  const NOTICES = {
    unpaired: ['Not paired', 'Open Remote devices on the Mac, allow pairing, and scan its code with this phone.'],
    removed: ['This device was removed on the host', 'Pair again from the Mac’s Remote devices window.'],
    changed: ['Identity changed', 'The host at this address is not the one this browser paired with. Pair again.'],
    unreachable: ['Can’t reach the host', 'Check that the Mac is awake, Switchboard is running, and it allows devices on the local network.'],
    cookieRefused: ['Paired, but this browser did not keep the sign-in', 'The Mac lists this device; remove it there. This browser refused the host’s cookie, so try another browser.'],
  };

  const PAIR_ERRORS = {
    pairing_closed: 'Pairing is not open on the Mac. Open it again and scan the new code.',
    wrong_code: 'The code did not match. Scan the code the Mac shows now.',
    wrong_key: 'The host refused the key. Scan the code again on the Mac you mean.',
    rate_limited: 'Too many attempts. Wait a minute and scan again.',
  };

  async function unauthorized() {
    stopUpdates();
    const saved = stored();
    let info = null;
    if (saved) {
      try { info = (await call(API.info)).body; } catch (_) { info = null; }
    }
    const [title, text] = NOTICES[inferUnauthorized(saved, info)];
    state.me = null;
    notice(title, text);
  }

  /**
   * A failed request: a 401 says why; no answer says so until a later
   * poll succeeds; any other error says so only before the list was
   * ever drawn, since otherwise the page would be left blank.
   */
  async function failed(error) {
    if (error instanceof HttpError && error.status === 401) return unauthorized();
    if (!(error instanceof HttpError)) return notice(...NOTICES.unreachable);
    if (!state.listMark) return notice('The host answered with an error', `${error.message}. Reload to try again.`);
    return null;
  }

  /**
   * Run `work` after every request queued before it: one at a time,
   * since requests in parallel would open new connections and the host
   * holds three from one address that have not yet authenticated before
   * it refuses a fourth. After pairing the calls ride one kept
   * connection; before, the host closes after each reply, so each is
   * its own. Work already in the queue calls the inner functions
   * directly, never `serial` again.
   */
  function serial(work) {
    const run = state.chain.then(() => work());
    state.chain = run.catch(() => null);
    return run;
  }

  /** The section the signed-in page is on: the open session or the list. */
  function showCurrent() {
    show(state.open ? 'session' : 'list');
  }

  async function pair(invite) {
    let info;
    try { info = (await call(API.info)).body; } catch (_) { return notice(...NOTICES.unreachable); }
    if (String(info.spki_sha256).toLowerCase() !== invite.key || String(info.server_id).toLowerCase() !== invite.id) {
      return notice('Wrong host', 'This address is a different Switchboard than the code was for. Nothing was sent.');
    }
    try {
      await call(API.pair, null, {
        code: invite.code,
        spki_sha256: invite.key,
        device_name: deviceName(navigator.userAgent, navigator.maxTouchPoints || 0),
        transport: 'cookie',
      });
    } catch (error) {
      if (error instanceof HttpError) return notice('Pairing failed', PAIR_ERRORS[error.code] || error.message);
      return notice(...NOTICES.unreachable);
    }
    // The host is remembered only once the cookie is shown to work: a
    // browser that refused it would otherwise read its next 401 as this
    // device removed on the host.
    try {
      state.me = (await call(API.me)).body;
    } catch (error) {
      if (error instanceof HttpError && error.status === 401) return notice(...NOTICES.cookieRefused);
      return failed(error);
    }
    localStorage.setItem(STORE_KEY, JSON.stringify({
      server_id: invite.id,
      spki_sha256: invite.key,
      server_name: info.server_name,
    }));
    return enter();
  }

  async function start() {
    try {
      state.me = (await call(API.me)).body;
    } catch (error) {
      return failed(error);
    }
    return enter();
  }

  /** Draw the signed-in page for `state.me` and keep it current. */
  async function enter() {
    $('host-name').textContent = state.me.server_name || 'Switchboard';
    $('scope').textContent = state.me.scope;
    await refreshList();
    startUpdates();
    return null;
  }

  function refreshList() {
    return serial(loadList);
  }

  async function loadList() {
    let projects;
    const sessionsByProject = {};
    let mark;
    try {
      const reply = await call(API.projects);
      mark = { epoch: reply.epoch, revision: reply.revision };
      projects = reply.body.projects || [];
      for (const p of projects) {
        const r = await call(API.sessions, p.id);
        sessionsByProject[p.id] = r.body.sessions || [];
      }
    } catch (error) {
      return failed(error);
    }
    if (!isCurrent(state.listMark, mark)) return null;
    state.listMark = mark;
    state.sessions = {};
    for (const list of Object.values(sessionsByProject)) {
      for (const s of list) state.sessions[s.id] = s;
    }
    state.loaded = { projects, sessionsByProject };
    drawProjectChoices();
    redraw();
    if (state.open) {
      if (state.sessions[state.open]) await loadSession();
      else closeSession();
    }
    return null;
  }

  /** The list arranged under the current choices, drawn. */
  function redraw() {
    const { projects, sessionsByProject } = state.loaded;
    drawList(arrangeSessions(projects, sessionsByProject, state.view, state.search));
  }

  /**
   * A checkbox per project, rebuilt only when the projects change, so a
   * poll does not take a tick from under the reader's finger.
   */
  function drawProjectChoices() {
    const projects = state.loaded.projects;
    const key = JSON.stringify(projects.map((p) => [p.id, p.name]));
    if (key !== state.projectChoices) {
      state.projectChoices = key;
      const root = $('f-project-list');
      root.replaceChildren();
      for (const project of projects) {
        const label = document.createElement('label');
        label.className = 'check';
        const box = document.createElement('input');
        box.type = 'checkbox';
        box.value = project.id;
        box.addEventListener('change', () => {
          state.view.projects = Array.from(root.querySelectorAll('input:checked'), (b) => b.value);
          viewChanged();
        });
        const name = document.createElement('span');
        name.textContent = project.name;
        label.append(box, name);
        root.append(label);
      }
    }
    syncControls();
  }

  /**
   * The controls showing `state.view`. The search box is left alone: it
   * is only ever what the reader typed, and setting it would move their
   * caret.
   */
  function syncControls() {
    $('f-state').value = state.view.state;
    $('f-kind').value = state.view.kind;
    $('f-sort').value = state.view.sort;
    const picked = new Set(state.view.projects);
    for (const box of $('f-project-list').querySelectorAll('input')) box.checked = picked.has(box.value);
    // The count keeps a folded band from narrowing the list unseen.
    const changed = changedCount(state.view, state.loaded.projects);
    $('filters-toggle').textContent = changed ? `Filters (${changed})` : 'Filters';
  }

  function viewChanged() {
    saveView();
    syncControls();
    redraw();
  }

  function showEverything() {
    state.view = { ...state.view, state: 'all', kind: 'all', projects: [] };
    state.search = '';
    $('search').value = '';
    viewChanged();
  }

  function toggleFilters() {
    const open = $('filters').hidden;
    $('filters').hidden = !open;
    $('filters-toggle').setAttribute('aria-expanded', String(open));
  }

  /** A session's kind, card and age, for its row and its page. */
  function metaText(s, now) {
    return [s.kind, s.card, sinceText(recencyMs(s) || null, now)].filter(Boolean).join(' · ');
  }

  /** The empty text, counts and hidden-waiting line above the rows. */
  function drawSummary(result) {
    $('list-empty').hidden = result.shown > 0;
    $('list-empty-text').textContent = emptyText(result, state.view, state.search);
    $('show-all').hidden = result.total === 0;
    $('list-count').textContent = `${result.shown} of ${result.total} shown`;
    const hidden = result.hiddenWaiting;
    $('list-hidden-waiting').hidden = hidden === 0;
    $('list-hidden-waiting').textContent = hidden === 1
      ? '1 session waiting on you is hidden by the filters'
      : `${hidden} sessions waiting on you are hidden by the filters`;
  }

  function drawList(result) {
    drawSummary(result);
    const root = $('sessions');
    root.replaceChildren();
    const now = Date.now();
    for (const { project, session: s } of result.rows) {
      const row = document.createElement('button');
      row.type = 'button';
      row.className = s.waiting ? 'row is-waiting' : 'row';
      if (s.waiting) {
        const w = document.createElement('p');
        w.className = 'waiting';
        w.textContent = s.waiting_reason ? `Waiting on you: ${s.waiting_reason}` : 'Waiting on you';
        row.append(w);
      }
      const name = document.createElement('div');
      name.className = 'name';
      name.textContent = s.name;
      const meta = document.createElement('div');
      meta.className = 'meta';
      meta.textContent = [project.name, metaText(s, now)].filter(Boolean).join(' · ');
      row.append(name, meta);
      row.addEventListener('click', () => openSession(s.id));
      root.append(row);
    }
    showCurrent();
  }

  function openSession(id) {
    if (state.open !== id) {
      $('reply-text').value = '';
      $('reply-status').textContent = '';
      $('session-text').textContent = '';
      state.pending = null;
      state.turns = [];
      state.resumeId = null;
      $('turn-list').replaceChildren();
      $('turns').hidden = true;
      $('earlier').hidden = true;
      $('earlier-error').textContent = '';
      state.scrollToEnd = true;
      clearConfirm();
    }
    state.open = id;
    show('session');
    serial(loadSession);
  }

  function closeSession() {
    state.open = null;
    clearConfirm();
    show('list');
  }

  /**
   * Cancels the pending confirming read. The mark goes with it: a read
   * the confirmation was to correct must not be skipped as current when
   * the session is opened again.
   */
  function clearConfirm() {
    if (state.confirm) clearTimeout(state.confirm);
    state.confirm = null;
    state.readMark = null;
  }

  async function loadSession() {
    const id = state.open;
    const s = state.sessions[id];
    if (!s) return null;
    $('session-name').textContent = s.name;
    $('session-meta').textContent = metaText(s, Date.now());
    $('session-waiting').hidden = !s.waiting;
    $('session-waiting').textContent = s.waiting_reason ? `Waiting on you: ${s.waiting_reason}` : 'Waiting on you';
    const canReply = state.me && state.me.scope === 'control';
    $('reply').hidden = !canReply || s.prompt_refusal != null;
    $('session-refusal').hidden = !canReply || s.prompt_refusal == null;
    $('session-refusal').textContent = s.prompt_refusal ? `No reply from here: ${s.prompt_refusal}` : '';
    if (s.kind === 'claude') {
      // Another session's print moves the list but not this entry; the
      // conversation it was read against is still the one drawn.
      if (state.readMark && state.readMark.key === entryMark(s).key) return null;
      return readLatest(id, s);
    }
    const key = `${API.screen} ${id}`;
    try {
      const reply = await call(API.screen, id);
      if (state.open !== id || !isCurrent(state.held[key], reply)) return null;
      state.held[key] = { epoch: reply.epoch, revision: reply.revision };
      showText(reply.body.text || '');
    } catch (error) {
      return blockFailed(id, error);
    }
    return null;
  }

  /**
   * The last turns of `id`'s conversation, read against its list entry
   * `entry` and marked once drawn, so the next refresh skips while the
   * entry stays put; a read that fails or is discarded leaves no mark.
   */
  async function readLatest(id, entry) {
    const previous = state.readMark;
    state.readMark = null;
    if (!(await loadConversation(id, null))) return null;
    state.readMark = entryMark(entry);
    // The hook may arrive before the transcript has the turn's last
    // line, and after a stop the entry goes quiet, so a turn's
    // boundary is read once more a moment later.
    if (turnBoundary(previous, state.readMark)) {
      const mark = state.readMark;
      clearConfirm();
      state.readMark = mark;
      state.confirm = setTimeout(() => {
        state.confirm = null;
        state.readMark = null;
        serial(loadSession);
      }, POLL_FAST_MS);
    }
    return null;
  }

  /**
   * A conversation page, the last turns or those before turn `before`,
   * merged into the turns held; true when it was drawn. A session that
   * has not started its conversation answers `no_conversation`, which is
   * said in the block and is not an error.
   */
  async function loadConversation(id, before) {
    const key = `${API.conversation} ${id}`;
    try {
      const reply = await call(API.conversation, id, undefined, conversationQuery(before));
      if (state.open !== id || !isCurrent(state.held[key], reply)) return false;
      state.held[key] = { epoch: reply.epoch, revision: reply.revision };
      // Another transcript (a new conversation under the same session)
      // shares no turn numbers with the one held.
      if (reply.body.resume_id !== state.resumeId) state.turns = [];
      state.resumeId = reply.body.resume_id;
      state.turns = mergeTurns(state.turns, reply.body);
      drawTurns();
      return true;
    } catch (error) {
      // A failed request for older turns must not hide the turns held.
      if (before && error instanceof HttpError && error.status !== 401) {
        if (state.open === id) $('earlier-error').textContent = error.message;
        return false;
      }
      await blockFailed(id, error);
      return false;
    }
  }

  async function loadEarlier() {
    const id = state.open;
    if (!id || state.turns.length === 0) return null;
    $('earlier').disabled = true;
    $('earlier-error').textContent = '';
    try {
      // The lowest turn is read when this request's turn in the queue
      // comes: a refresh queued before it may have replaced the turns.
      await serial(async () => {
        if (state.open !== id) return null;
        const lowest = earlierBefore(state.turns);
        if (lowest == null) return null;
        const keepPlace = document.documentElement.scrollHeight;
        await loadConversation(id, lowest);
        // The older turns land above the reader's place; keep the place.
        window.scrollBy(0, document.documentElement.scrollHeight - keepPlace);
        return null; // serial's callers all resolve to null
      });
    } finally {
      $('earlier').disabled = false;
    }
    return null;
  }

  /** The screen, or the host's message, in the session's text block. */
  function showText(text) {
    $('turns').hidden = true;
    $('session-text').hidden = false;
    $('session-text').textContent = text;
  }

  /** A read for the open session failed: a 401 says why, anything else in the block. */
  async function blockFailed(id, error) {
    if (error instanceof HttpError && error.status !== 401) {
      if (state.open === id) showText(error.message);
      return null;
    }
    return failed(error);
  }

  /** The held turns, oldest first; a turn's steps stay open across a redraw. */
  function drawTurns() {
    const list = $('turn-list');
    const open = new Set(Array.from(list.querySelectorAll('details[open]'), (d) => d.dataset.n));
    const now = Date.now();
    list.replaceChildren();
    for (const turn of state.turns) {
      const article = document.createElement('article');
      article.className = 'turn';
      const meta = document.createElement('p');
      meta.className = 'meta';
      meta.textContent = [`Turn ${turn.n}`, sinceText(turn.at_ms, now)].filter(Boolean).join(' · ');
      article.append(meta);
      if (turn.user) {
        const prompt = document.createElement('div');
        prompt.className = 'prompt';
        prompt.textContent = turn.user;
        article.append(prompt);
      }
      const activity = turn.activity || [];
      if (activity.length > 0) {
        const details = document.createElement('details');
        details.dataset.n = String(turn.n);
        details.open = open.has(details.dataset.n);
        const summary = document.createElement('summary');
        summary.textContent = activity.length === 1 ? '1 step' : `${activity.length} steps`;
        const steps = document.createElement('ol');
        for (const a of activity) {
          const step = document.createElement('li');
          if (a.error) step.className = 'is-error';
          step.textContent = a.line;
          steps.append(step);
        }
        details.append(summary, steps);
        article.append(details);
      }
      if (turn.final_text) {
        const answer = document.createElement('div');
        answer.className = 'answer';
        answer.textContent = turn.final_text;
        article.append(answer);
      }
      list.append(article);
    }
    $('turns-empty').hidden = state.turns.length > 0;
    $('earlier').hidden = state.turns.length === 0 || state.turns[0].n <= 1;
    $('session-text').hidden = true;
    $('turns').hidden = false;
    // The view goes to the last turn once, when the session is opened;
    // a refresh leaves the reader where they are.
    if (state.scrollToEnd && list.lastElementChild) {
      state.scrollToEnd = false;
      list.lastElementChild.scrollIntoView({ block: 'start' });
    }
  }

  function newOp() {
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
  }

  async function send() {
    const id = state.open;
    const text = $('reply-text').value.trim();
    if (!id || !text) return null;
    // The same text to the same session keeps its op, so a retry after
    // a lost reply is answered from the host's log, not typed twice.
    if (!state.pending || state.pending.session !== id || state.pending.text !== text) {
      state.pending = { session: id, text, op: newOp() };
    }
    $('send').disabled = true;
    $('reply-status').textContent = 'Sending…';
    try {
      await call(API.send, id, { op: state.pending.op, text });
      state.pending = null;
      // Read after the await: what was typed mid-flight stays. An unchanged
      // box is not rewritten, because setting `.value` moves the caret.
      const box = $('reply-text');
      const rest = afterSend(box.value, text);
      if (rest !== box.value) box.value = rest;
      $('reply-status').textContent = 'Sent.';
    } catch (error) {
      if (error instanceof HttpError && error.status === 403 && error.code === 'scope') {
        $('reply-status').textContent = 'This device reads only.';
        try { state.me = (await call(API.me)).body; $('scope').textContent = state.me.scope; } catch (_) { /* the next poll says */ }
        await serial(loadSession);
      } else if (error instanceof HttpError && error.status === 401) {
        await unauthorized();
      } else if (error instanceof HttpError) {
        $('reply-status').textContent = error.message;
      } else {
        $('reply-status').textContent = 'Not sure it went. The box still holds it: send it again unchanged and it will not be typed twice.';
      }
    } finally {
      $('send').disabled = false;
    }
    return null;
  }

  // --- updates: a poll, hastened by the events socket when it connects

  async function poll() {
    // A visibility change calls this while a timer is still pending;
    // clearing it here keeps to one polling chain.
    if (state.timer) clearTimeout(state.timer);
    state.timer = null;
    if (document.visibilityState === 'visible' && state.me) {
      await serial(async () => {
        try {
          const reply = await call(API.waiting);
          const mark = state.listMark;
          if (!mark || mark.epoch !== reply.epoch || mark.revision !== reply.revision) {
            await loadList();
          } else {
            // Nothing changed, but an earlier failed poll may have put up
            // "Can't reach the host"; the answer just now takes it down.
            showCurrent();
          }
        } catch (error) {
          await failed(error);
        }
      });
    }
    if (state.me) schedule();
  }

  function schedule() {
    if (state.timer) clearTimeout(state.timer);
    state.timer = setTimeout(poll, state.pollMs);
  }

  function startUpdates() {
    state.pollMs = POLL_FAST_MS;
    schedule();
    openSocket();
  }

  function stopUpdates() {
    if (state.timer) clearTimeout(state.timer);
    state.timer = null;
    if (state.socket) {
      state.socket.onclose = null;
      state.socket.close();
      state.socket = null;
    }
  }

  function openSocket() {
    if (!state.me || state.socket) return;
    let socket;
    try {
      socket = new WebSocket(`wss://${location.host}${route(API.events).path}`);
    } catch (_) {
      return;
    }
    state.socket = socket;
    socket.onopen = () => {
      state.socketRetry = SOCKET_RETRY_MS;
      state.pollMs = POLL_SLOW_MS;
      schedule();
    };
    socket.onmessage = (event) => {
      let frame;
      try { frame = JSON.parse(event.data); } catch (_) { return; }
      if (frame.type === 'changed' || frame.type === 'removed' || frame.type === 'resync_required') {
        refreshList();
      }
    };
    socket.onclose = () => {
      state.socket = null;
      state.pollMs = POLL_FAST_MS;
      refreshList();
      schedule();
      const wait = state.socketRetry;
      state.socketRetry = Math.min(state.socketRetry * 2, SOCKET_RETRY_MAX_MS);
      setTimeout(openSocket, wait);
    };
  }

  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && state.me) poll();
  });

  document.addEventListener('DOMContentLoaded', () => {
    $('back').addEventListener('click', closeSession);
    $('send').addEventListener('click', send);
    $('earlier').addEventListener('click', loadEarlier);
    state.view = storedView();
    syncControls();
    $('filters-toggle').addEventListener('click', toggleFilters);
    $('show-all').addEventListener('click', showEverything);
    $('search').addEventListener('input', () => {
      state.search = $('search').value;
      redraw();
    });
    for (const [id, field] of [['f-state', 'state'], ['f-kind', 'kind'], ['f-sort', 'sort']]) {
      $(id).addEventListener('change', () => {
        state.view = readView({ ...state.view, [field]: $(id).value });
        viewChanged();
      });
    }
    const invite = parseFragment(location.hash);
    // The code leaves the address bar before anything else happens.
    if (location.hash) history.replaceState(null, '', location.pathname);
    if (invite) pair(invite);
    else start();
  });
}
