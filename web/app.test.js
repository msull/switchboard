// The page's pure parts, run with `node --test web/app.test.js` (the
// host's test suite runs it when node is installed).
'use strict';

const test = require('node:test');
const assert = require('node:assert');
const page = require('./app.js');

test('the fragment carries the invite, or nothing', () => {
  assert.deepStrictEqual(
    page.parseFragment('#code=ABCD2345&key=AB12&id=17A9B824-AC51-47DC-A96A-C2B9E69C85B4'),
    { code: 'ABCD2345', key: 'ab12', id: '17a9b824-ac51-47dc-a96a-c2b9e69c85b4' },
  );
  assert.strictEqual(page.parseFragment(''), null);
  assert.strictEqual(page.parseFragment('#key=ab&id=x'), null);
  assert.strictEqual(page.parseFragment('#code=%E0%A4%A&key=ab&id=x'), null);
});

test('the device name comes from the user agent', () => {
  const iphone = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1';
  const ipad = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15';
  const chromeAndroid = 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0 Mobile Safari/537.36';
  const edge = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0 Safari/537.36 Edg/129.0';
  assert.strictEqual(page.deviceName(iphone, 5), 'Safari on iPhone');
  assert.strictEqual(page.deviceName(ipad, 5), 'Safari on iPad');
  assert.strictEqual(page.deviceName(ipad, 0), 'Safari on Mac');
  assert.strictEqual(page.deviceName(chromeAndroid, 5), 'Chrome on Android');
  assert.strictEqual(page.deviceName(edge, 0), 'Edge on Windows');
  assert.strictEqual(page.deviceName('curl/8', 0), 'Browser');
});

test('a 401 is told apart by what /info says', () => {
  const stored = { server_id: 'a', spki_sha256: 'k' };
  assert.strictEqual(page.inferUnauthorized(null, null), 'unpaired');
  assert.strictEqual(page.inferUnauthorized(stored, null), 'unreachable');
  assert.strictEqual(page.inferUnauthorized(stored, { server_id: 'A', spki_sha256: 'K' }), 'removed');
  assert.strictEqual(page.inferUnauthorized(stored, { server_id: 'a', spki_sha256: 'other' }), 'changed');
  assert.strictEqual(page.inferUnauthorized(stored, { server_id: 'b', spki_sha256: 'k' }), 'changed');
});

test('a reply older than the one held is dropped', () => {
  const held = { epoch: 'e', revision: 5 };
  assert.ok(page.isCurrent(null, held));
  assert.ok(page.isCurrent(held, { epoch: 'e', revision: 5 }));
  assert.ok(page.isCurrent(held, { epoch: 'e', revision: 6 }));
  assert.ok(!page.isCurrent(held, { epoch: 'e', revision: 4 }));
  assert.ok(page.isCurrent(held, { epoch: 'restarted', revision: 1 }));
});

const projects = [{ id: 'p1', name: 'beta' }, { id: 'p2', name: 'alpha' }, { id: 'p3', name: 'empty' }];
const s = (id, fields) => ({ id, name: id, kind: 'claude', liveness: 'running', card: 'idle', waiting: false, ...fields });
const fixture = () => ({
  p1: [
    s('a', { last_activity_ms: 100 }),
    s('Build', { kind: 'shell', waiting: true, last_activity_ms: 50 }),
    s('gone', { liveness: { exited: { code: 0 } }, card: 'exited (0)', last_seen_ms: 900 }),
  ],
  p2: [
    s('c', { card: 'starting', last_activity_ms: 300 }),
    s('lost', { liveness: 'missing', card: 'not running', waiting: true, last_seen_ms: 10 }),
  ],
});
const view = (fields) => ({ ...page.readView(null), ...fields });
const ids = (result) => result.rows.map((r) => r.session.id);

test('the default list is every live session, waiting first, then the most recent', () => {
  const result = page.arrangeSessions(projects, fixture(), page.readView(null), '');
  assert.deepStrictEqual(ids(result), ['Build', 'c', 'a']);
  assert.deepStrictEqual(result.rows.map((r) => r.project.id), ['p1', 'p2', 'p1']);
  assert.strictEqual(result.shown, 3);
  assert.strictEqual(result.total, 5);
  assert.strictEqual(result.hiddenWaiting, 1, 'the missing waiting session is hidden');
  assert.ok(page.isLive(s('x')));
  assert.ok(!page.isLive(s('x', { liveness: 'missing' })));
  assert.ok(!page.isLive(s('x', { liveness: { exited: { code: null } } })));
});

test('the state, kind and project choices narrow the list', () => {
  const by = (fields, search) => ids(page.arrangeSessions(projects, fixture(), view(fields), search));
  assert.deepStrictEqual(by({ state: 'waiting' }), ['Build', 'lost']);
  assert.deepStrictEqual(by({ state: 'ended' }), ['lost', 'gone']);
  assert.strictEqual(by({ state: 'all' }).length, 5);
  assert.deepStrictEqual(by({ state: 'all', kind: 'shell' }), ['Build']);
  assert.deepStrictEqual(by({ state: 'all', projects: ['p2'] }), ['lost', 'c']);
  assert.strictEqual(by({ state: 'all', projects: ['stale'] }).length, 5, 'a stale project means all');
  assert.deepStrictEqual(by({ state: 'all', projects: ['stale', 'p2'] }), ['lost', 'c']);
  assert.strictEqual(by({ state: 'all', projects: [] }).length, 5);
});

test('the search matches names, trimmed and in any case', () => {
  const by = (search) => ids(page.arrangeSessions(projects, fixture(), view({ state: 'all' }), search));
  assert.deepStrictEqual(by('  bUiL '), ['Build']);
  assert.deepStrictEqual(by('o'), ['lost', 'gone']);
  assert.deepStrictEqual(by('shell'), [], 'the kind is not searched');
  assert.strictEqual(by('   ').length, 5);
});

test('each sort orders the rows, the host order breaking ties', () => {
  const by = (sort) => ids(page.arrangeSessions(projects, fixture(), view({ state: 'all', sort }), ''));
  assert.deepStrictEqual(by('waiting'), ['Build', 'lost', 'gone', 'c', 'a']);
  assert.deepStrictEqual(by('recent'), ['gone', 'c', 'a', 'Build', 'lost'], 'an ended session sorts by when it was last seen');
  assert.deepStrictEqual(by('project'), ['c', 'lost', 'gone', 'a', 'Build']);
  assert.deepStrictEqual(by('name'), ['a', 'Build', 'c', 'gone', 'lost']);
  const twins = { p1: [s('x', { name: 'same' }), s('y', { name: 'same' }), s('z', { name: 'same' })] };
  for (const sort of ['waiting', 'recent', 'project', 'name']) {
    assert.deepStrictEqual(ids(page.arrangeSessions(projects, twins, view({ sort }), '')), ['x', 'y', 'z']);
  }
  assert.strictEqual(page.recencyMs({ last_activity_ms: 5, last_seen_ms: 9, last_stop_at_ms: 7 }), 9);
  assert.strictEqual(page.recencyMs({ last_activity_ms: null, last_seen_ms: undefined, last_stop_at_ms: 3 }), 3);
  assert.strictEqual(page.recencyMs({}), 0);
});

test('a waiting session hidden by any choice is counted', () => {
  const hidden = (fields, search) => page.arrangeSessions(projects, fixture(), view(fields), search).hiddenWaiting;
  assert.strictEqual(hidden({ state: 'all' }, ''), 0);
  assert.strictEqual(hidden({ state: 'ended' }, ''), 1, 'by state');
  assert.strictEqual(hidden({ state: 'all', kind: 'claude' }, ''), 1, 'by kind');
  assert.strictEqual(hidden({ state: 'all', projects: ['p2'] }, ''), 1, 'by project');
  assert.strictEqual(hidden({ state: 'all' }, 'gone'), 2, 'by search');
});

test('a stored view is read back, anything odd as the defaults', () => {
  const defaults = { state: 'live', kind: 'all', projects: [], sort: 'waiting' };
  assert.deepStrictEqual(page.readView(null), defaults);
  assert.deepStrictEqual(page.readView('live'), defaults);
  assert.deepStrictEqual(page.readView([1]), defaults);
  assert.deepStrictEqual(page.readView({ state: 'bogus', kind: 3, projects: 'p1', sort: null }), defaults);
  assert.deepStrictEqual(page.readView({ projects: ['p1', 7] }).projects, ['p1']);
  const kept = { state: 'ended', kind: 'codex', projects: ['p1'], sort: 'name' };
  assert.deepStrictEqual(page.readView(JSON.parse(JSON.stringify(kept))), kept);
  assert.deepStrictEqual({ ...page.DEFAULT_VIEW, projects: [...page.DEFAULT_VIEW.projects] }, defaults);
});

test('an empty list says why', () => {
  const none = page.arrangeSessions(projects, {}, page.readView(null), '');
  assert.strictEqual(page.emptyText(none, page.readView(null), ''), 'No sessions.');
  const cold = { p1: [s('a', { liveness: 'missing' }), s('b', { liveness: 'missing' })] };
  const result = page.arrangeSessions(projects, cold, page.readView(null), '');
  assert.strictEqual(result.shown, 0);
  assert.strictEqual(page.emptyText(result, page.readView(null), ''), 'No live sessions; 2 not running.');
  assert.strictEqual(page.emptyText(result, view({ kind: 'shell' }), ''), 'Nothing matches these filters.');
  assert.strictEqual(page.emptyText(result, page.readView(null), 'zz'), 'Nothing matches these filters.');
});

test('the filters button counts the choices that differ from the defaults', () => {
  assert.strictEqual(page.changedCount(page.readView(null), projects), 0);
  assert.strictEqual(page.changedCount(view({ state: 'all', sort: 'name' }), projects), 2);
  assert.strictEqual(page.changedCount(view({ kind: 'shell', projects: ['p1', 'p2'] }), projects), 2);
  assert.strictEqual(page.changedCount(view({ projects: ['gone'] }), projects), 0);
});

test('times read as a person says them', () => {
  const now = 10 * 86400 * 1000;
  assert.strictEqual(page.sinceText(null, now), '');
  assert.strictEqual(page.sinceText(now - 5000, now), 'just now');
  assert.strictEqual(page.sinceText(now - 5 * 60000, now), '5 min ago');
  assert.strictEqual(page.sinceText(now - 3 * 3600000, now), '3 h ago');
  assert.strictEqual(page.sinceText(now - 2 * 86400000, now), '2 d ago');
});

test('a route fills its id', () => {
  assert.deepStrictEqual(page.route(page.API.send, 'x y'), { method: 'POST', path: '/api/v1/sessions/x%20y/send' });
  assert.deepStrictEqual(page.route(page.API.info), { method: 'GET', path: '/api/v1/info' });
});

test('a conversation query asks for the last turns or those before one', () => {
  assert.strictEqual(page.conversationQuery(null), '?from=last&limit=20');
  assert.strictEqual(page.conversationQuery(7), '?before=7&limit=20');
});

test('turns merge by number', () => {
  const turn = (n, text) => ({ n, final_text: text || `answer ${n}` });
  const held = [turn(3), turn(4)];
  const ns = (turns) => turns.map((t) => t.n);

  const replaced = page.mergeTurns(held, { total: 5, turns: [turn(4, 'new'), turn(5)] });
  assert.deepStrictEqual(ns(replaced), [3, 4, 5]);
  assert.strictEqual(replaced[1].final_text, 'new');

  const gap = page.mergeTurns(held, { total: 9, turns: [turn(8), turn(9)] });
  assert.deepStrictEqual(ns(gap), [8, 9], 'a gap drops the held turns');

  const earlier = page.mergeTurns(held, { total: 4, turns: [turn(1), turn(2)] });
  assert.deepStrictEqual(ns(earlier), [1, 2, 3, 4]);

  const same = { total: 4, turns: [turn(3), turn(4)] };
  assert.deepStrictEqual(page.mergeTurns(page.mergeTurns(held, same), same), page.mergeTurns(held, same));

  assert.deepStrictEqual(ns(page.mergeTurns(held, { total: 3, turns: [] })), [3], 'past the total is gone');
  assert.deepStrictEqual(ns(page.mergeTurns([], { total: 2, turns: [turn(1), turn(2)] })), [1, 2]);
});

test('the reply box keeps what was typed after a send', () => {
  assert.strictEqual(page.afterSend('second', 'second'), '');
  assert.strictEqual(page.afterSend('second extra', 'second'), ' extra');
  assert.strictEqual(page.afterSend('  second extra', 'second'), ' extra');
  assert.strictEqual(page.afterSend('second\n', 'second'), '');
  assert.strictEqual(page.afterSend('seco extra', 'second'), 'seco extra', 'an edited box is kept whole');
  assert.strictEqual(page.afterSend('', 'second'), '');
  assert.strictEqual(page.afterSend('secondary', 'second'), 'ary', 'the rule is a prefix rule');
});
