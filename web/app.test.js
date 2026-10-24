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

test('waiting sessions and their projects come first', () => {
  const projects = [{ id: 'p1', name: 'one' }, { id: 'p2', name: 'two' }, { id: 'p3', name: 'empty' }];
  const groups = page.orderSessions(projects, {
    p1: [{ id: 'a', waiting: false }, { id: 'b', waiting: false }],
    p2: [{ id: 'c', waiting: false }, { id: 'd', waiting: true }],
  });
  assert.deepStrictEqual(groups.map((g) => g.project.id), ['p2', 'p1']);
  assert.deepStrictEqual(groups[0].sessions.map((s) => s.id), ['d', 'c']);
  assert.deepStrictEqual(groups[1].sessions.map((s) => s.id), ['a', 'b']);
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
