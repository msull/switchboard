// Switchboard's web page: pair a browser from the pairing window's QR,
// list the sessions with the waiting ones first, read one, and reply.
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
  lastResponse: 'GET /api/v1/sessions/{id}/last-response',
  screen: 'GET /api/v1/sessions/{id}/screen',
  send: 'POST /api/v1/sessions/{id}/send',
  events: 'GET /api/v1/events',
};

const STORE_KEY = 'switchboard-host';
const POLL_FAST_MS = 3000;
const POLL_SLOW_MS = 20000;
const SOCKET_RETRY_MS = 5000;
const SOCKET_RETRY_MAX_MS = 60000;

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

/**
 * The projects with their sessions, a project with a waiting session
 * first and waiting sessions first within each, the host's order kept
 * otherwise. Projects with no session are left out.
 */
function orderSessions(projects, sessionsByProject) {
  const waitingFirst = (list) => list
    .map((item, i) => ({ item, i }))
    .sort((a, b) => (b.item.waiting - a.item.waiting) || (a.i - b.i))
    .map((x) => x.item);
  const groups = projects
    .map((project) => ({
      project,
      sessions: waitingFirst((sessionsByProject[project.id] || []).map((s) => ({ ...s, waiting: !!s.waiting }))),
    }))
    .filter((g) => g.sessions.length > 0)
    .map((g) => ({ ...g, waiting: g.sessions.some((s) => s.waiting) }));
  return waitingFirst(groups).map(({ project, sessions }) => ({ project, sessions }));
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

if (typeof module !== 'undefined' && module.exports) {
  module.exports = { API, parseFragment, deviceName, inferUnauthorized, isCurrent, orderSessions, sinceText, route };
}

// --- the page

if (typeof document !== 'undefined') {
  const $ = (id) => document.getElementById(id);
  const state = {
    me: null,
    listMark: null,
    open: null,
    sessions: {},
    held: {},
    pending: null,
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

  async function call(entry, id, body) {
    const { method, path } = route(entry, id);
    const init = { method, credentials: 'same-origin', cache: 'no-store', headers: {} };
    if (body !== undefined) {
      init.headers['Content-Type'] = 'application/json';
      init.body = JSON.stringify(body);
    }
    const response = await fetch(path, init);
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

  async function refreshList() {
    let projects;
    const sessionsByProject = {};
    let mark;
    try {
      const reply = await call(API.projects);
      mark = { epoch: reply.epoch, revision: reply.revision };
      projects = reply.body.projects || [];
      // One at a time, on the one kept connection: requests in
      // parallel would open new connections, and the host holds four
      // that have not yet authenticated before it refuses a fifth.
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
    drawList(orderSessions(projects, sessionsByProject));
    if (state.open) {
      if (state.sessions[state.open]) await refreshSession();
      else closeSession();
    }
    return null;
  }

  function drawList(groups) {
    const root = $('projects');
    root.replaceChildren();
    $('list-empty').hidden = groups.length > 0;
    const now = Date.now();
    for (const { project, sessions } of groups) {
      const heading = document.createElement('h3');
      heading.textContent = project.name;
      root.append(heading);
      for (const s of sessions) {
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
        meta.textContent = [s.kind, s.card, sinceText(s.last_activity_ms, now)].filter(Boolean).join(' · ');
        row.append(name, meta);
        row.addEventListener('click', () => openSession(s.id));
        root.append(row);
      }
    }
    showCurrent();
  }

  function openSession(id) {
    if (state.open !== id) {
      $('reply-text').value = '';
      $('reply-status').textContent = '';
      $('session-text').textContent = '';
      state.pending = null;
    }
    state.open = id;
    show('session');
    refreshSession();
  }

  function closeSession() {
    state.open = null;
    show('list');
  }

  async function refreshSession() {
    const id = state.open;
    const s = state.sessions[id];
    if (!s) return null;
    $('session-name').textContent = s.name;
    $('session-meta').textContent = [s.kind, s.card, sinceText(s.last_activity_ms, Date.now())].filter(Boolean).join(' · ');
    $('session-waiting').hidden = !s.waiting;
    $('session-waiting').textContent = s.waiting_reason ? `Waiting on you: ${s.waiting_reason}` : 'Waiting on you';
    const canReply = state.me && state.me.scope === 'control';
    $('reply').hidden = !canReply || s.prompt_refusal != null;
    $('session-refusal').hidden = !canReply || s.prompt_refusal == null;
    $('session-refusal').textContent = s.prompt_refusal ? `No reply from here: ${s.prompt_refusal}` : '';
    const entry = s.kind === 'claude' ? API.lastResponse : API.screen;
    const key = `${entry} ${id}`;
    try {
      const reply = await call(entry, id);
      if (state.open !== id || !isCurrent(state.held[key], reply)) return null;
      state.held[key] = { epoch: reply.epoch, revision: reply.revision };
      $('session-text').textContent = reply.body.text || '';
    } catch (error) {
      if (error instanceof HttpError && error.status !== 401) {
        if (state.open === id) $('session-text').textContent = error.message;
        return null;
      }
      return failed(error);
    }
    return null;
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
      $('reply-text').value = '';
      $('reply-status').textContent = 'Sent.';
    } catch (error) {
      if (error instanceof HttpError && error.status === 403 && error.code === 'scope') {
        $('reply-status').textContent = 'This device reads only.';
        try { state.me = (await call(API.me)).body; $('scope').textContent = state.me.scope; } catch (_) { /* the next poll says */ }
        await refreshSession();
      } else if (error instanceof HttpError && error.status === 401) {
        await unauthorized();
      } else if (error instanceof HttpError) {
        $('reply-status').textContent = error.message;
      } else {
        $('reply-status').textContent = 'Not sure it went. Send again: it will not be typed twice.';
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
      try {
        const reply = await call(API.waiting);
        const mark = state.listMark;
        if (!mark || mark.epoch !== reply.epoch || mark.revision !== reply.revision) {
          await refreshList();
        } else {
          // Nothing changed, but an earlier failed poll may have put up
          // "Can't reach the host"; the answer just now takes it down.
          showCurrent();
        }
      } catch (error) {
        await failed(error);
      }
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
    const invite = parseFragment(location.hash);
    // The code leaves the address bar before anything else happens.
    if (location.hash) history.replaceState(null, '', location.pathname);
    if (invite) pair(invite);
    else start();
  });
}
