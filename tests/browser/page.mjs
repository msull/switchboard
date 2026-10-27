// The browser half of `the_page_works_in_a_real_browser` in
// tests/remote.rs: drives the web page in headless Chromium against the
// test's host. Before each step it prints `STEP <name>` and waits for a
// line on stdin, the host's answer once it has done that step's part:
// `go`, or `invite <url>` for a pairing window it opened. The host is
// served only while the Rust side pumps it, so this side never runs
// ahead of an answer. The first failed check prints `FAIL <step>:` and
// exits 1; a full run ends with `DONE`. The `socket:` and
// `polls in 9 s:` lines are printed measurements, not checks.
import assert from 'node:assert/strict';
import readline from 'node:readline';
import { chromium } from 'playwright';

const lines = readline.createInterface({ input: process.stdin })[Symbol.asyncIterator]();
let current = 'start';

async function step(name) {
  current = name;
  console.log(`STEP ${name}`);
  const { value, done } = await lines.next();
  if (done) throw new Error('the host closed stdin');
  return value;
}

async function invite(name) {
  const answer = await step(name);
  assert.ok(answer.startsWith('invite '), `expected an invite, got ${answer}`);
  return answer.slice('invite '.length);
}

const text = (page, selector) => page.locator(selector).textContent();

/** Waits until `check()` holds, polling, or fails saying `what`. */
async function until(check, what, ms = 15000) {
  const end = Date.now() + ms;
  while (!(await check())) {
    if (Date.now() > end) throw new Error(`timed out: ${what}`);
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
}

/** The turn numbers drawn, from each turn's `Turn N` line. */
const turnNumbers = (page) => page.$$eval('#turn-list .turn > .meta', (metas) => metas.map((m) => Number(m.textContent.match(/^Turn (\d+)/)[1])));

/** A promise that resolves on `release()`, for holding a request. */
function gate() {
  let release;
  const held = new Promise((resolve) => { release = resolve; });
  return { held, release };
}

/** No horizontal scroll: what a phone shows is the page's width. */
async function fitsWidth(page) {
  const [scroll, inner] = await page.evaluate(() => [document.documentElement.scrollWidth, window.innerWidth]);
  assert.ok(scroll <= inner, `scrollWidth ${scroll} > innerWidth ${inner}`);
}

/** Records the page's events socket, so the run can say whether it opened. */
function recordSockets() {
  window.__sockets = [];
  const Real = window.WebSocket;
  window.WebSocket = class extends Real {
    constructor(...args) {
      super(...args);
      for (const kind of ['open', 'message', 'close']) this.addEventListener(kind, () => window.__sockets.push(kind));
    }
  };
}

async function noticeTitle(page, title) {
  await page.locator('#notice-title', { hasText: title }).waitFor({ timeout: 15000 });
  assert.equal(await text(page, '#notice-title'), title);
}

/** A request that got no answer, such as a load past the host's pool of four. */
function reportFailures(ctx) {
  ctx.on('requestfailed', (r) => console.log(`requestfailed ${new URL(r.url()).pathname}: ${r.failure()?.errorText}`));
}

const browser = await chromium.launch();
try {
  // 1. A browser that refuses the cookie is told so, not that the
  //    device was removed, and the host is not remembered.
  {
    const url = await invite('refused-cookie');
    const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
    reportFailures(ctx);
    await ctx.route('**/api/v1/pair', async (route) => {
      const response = await route.fetch();
      // Whatever the fetch stored, the browser keeps no cookie.
      await ctx.clearCookies();
      const headers = { ...response.headers() };
      delete headers['set-cookie'];
      await route.fulfill({ status: response.status(), headers, body: await response.body() });
    });
    const page = await ctx.newPage();
    await page.goto(url);
    await noticeTitle(page, 'Paired, but this browser did not keep the sign-in');
    assert.equal(await page.evaluate(() => localStorage.getItem('switchboard-host')), null);
    await ctx.close();
  }

  // 2. Pairing: the fragment leaves the address bar, the host is
  //    remembered, the cookie is one script never reads, the list is
  //    drawn at phone width, and a chosen sort survives a reload.
  const url = await invite('pair');
  const serverId = new URL(url).hash.match(/id=([^&]+)/)[1];
  const ctx = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 390, height: 844 } });
  reportFailures(ctx);
  await ctx.addInitScript(recordSockets);
  const page = await ctx.newPage();
  await page.goto(url);
  const row = page.locator('#sessions .row', { hasText: 'investigator' });
  await row.waitFor({ timeout: 15000 });
  assert.equal(await page.evaluate(() => location.hash), '');
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('switchboard-host')));
  assert.equal(saved.server_id, serverId);
  const cookie = (await ctx.cookies()).find((c) => c.name === `__Host-sb-${serverId}`);
  assert.ok(cookie, 'the sign-in cookie is stored');
  assert.deepEqual([cookie.secure, cookie.httpOnly, cookie.sameSite], [true, true, 'Strict']);
  assert.ok(!(await page.evaluate(() => document.cookie)).includes('__Host-sb-'), 'script cannot read the cookie');
  await fitsWidth(page);
  await page.click('#filters-toggle');
  await page.selectOption('#f-sort', 'name');
  await page.reload();
  await row.waitFor({ timeout: 15000 });
  assert.equal(await page.inputValue('#f-sort'), 'name');

  // 3. Opening the session shows its conversation.
  await step('open');
  await row.click();
  await page.locator('#turn-list', { hasText: 'prompt 3' }).waitFor({ timeout: 15000 });
  assert.ok((await text(page, '#turn-list')).includes('the token is tok-5e3cr3t'));
  await fitsWidth(page);

  // 4. A poll that redraws the list and the open session leaves what
  //    the reader is typing alone.
  current = 'reply-box';
  assert.ok(await page.isVisible('#reply'), `no reply box: ${await text(page, '#session-refusal')}`);
  await page.fill('#reply-text', 'draft');
  await step('poll-keeps-reply');
  await page.locator('#turn-list', { hasText: 'prompt 4' }).waitFor({ timeout: 15000 });
  await page.locator('#session-name', { hasText: 'investigator 2' }).waitFor({ timeout: 15000 });
  assert.ok((await text(page, '#sessions')).includes('investigator 2'), 'the list was redrawn');
  assert.equal(await page.inputValue('#reply-text'), 'draft');
  const sockets = await page.evaluate(() => window.__sockets);
  console.log(`socket: ${sockets.includes('open') ? 'opened' : 'refused'}`);

  // 4a. "Earlier turns" asks for the turns before the lowest held when
  //     its turn in the queue comes: a list refresh queued ahead of it
  //     that replaced the turns leaves no gap.
  await step('grow-45');
  await page.locator('#turn-list', { hasText: 'prompt 45' }).waitFor({ timeout: 15000 });
  assert.deepEqual(await turnNumbers(page), Array.from({ length: 20 }, (_, i) => 26 + i));
  assert.ok(await page.isVisible('#earlier'), 'the Earlier button shows');
  {
    // Held requests reach the host only once released, after its step.
    const list = gate();
    let reached;
    const seen = new Promise((resolve) => { reached = resolve; });
    await page.route('**/api/v1/projects', async (route) => {
      reached();
      await list.held;
      await route.continue();
    });
    await step('grow-70');
    await seen;
    await page.click('#earlier');
    list.release();
    await until(async () => (await turnNumbers(page)).length > 20, 'the earlier page merged');
    await page.unroute('**/api/v1/projects');
    const ns = await turnNumbers(page);
    assert.equal(ns[ns.length - 1], 70);
    ns.forEach((n, i) => assert.equal(n, ns[0] + i, `turns ${ns.join(',')} are consecutive`));
  }

  // 4b. A switch clears the conversation's chrome: no Earlier button
  //     over the next session's screen while it loads, and no error
  //     left from the last one.
  {
    const screen = gate();
    let reached;
    const seen = new Promise((resolve) => { reached = resolve; });
    await page.route('**/api/v1/sessions/*/screen', async (route) => {
      reached();
      await screen.held;
      await route.continue();
    });
    await page.click('#back');
    await page.locator('#sessions .row', { hasText: 'shell' }).click();
    await seen;
    assert.ok(await page.isHidden('#turns'), 'the turns hide while the screen loads');
    assert.ok(await page.isHidden('#earlier'), 'the Earlier button hides while the screen loads');
    screen.release();
    await page.locator('#session-text').waitFor({ state: 'visible', timeout: 15000 });
    await page.unroute('**/api/v1/sessions/*/screen');

    await page.click('#back');
    await page.locator('#sessions .row', { hasText: 'investigator' }).click();
    await page.locator('#turn-list', { hasText: 'prompt 70' }).waitFor({ timeout: 15000 });
    const older = (u) => u.pathname.endsWith('/conversation') && u.searchParams.has('before');
    await page.route(older, (route) => route.fulfill({
      status: 500,
      contentType: 'application/json',
      body: JSON.stringify({ error: { code: 'failed', message: 'earlier failed' } }),
    }));
    await page.click('#earlier');
    await page.locator('#earlier-error', { hasText: 'earlier failed' }).waitFor({ timeout: 15000 });
    await page.unroute(older);
    await page.click('#back');
    await page.locator('#sessions .row', { hasText: 'shell' }).click();
    assert.equal(await text(page, '#earlier-error'), '');
    await page.click('#back');
    await page.locator('#sessions .row', { hasText: 'investigator' }).click();
    await page.locator('#turn-list', { hasText: 'prompt 70' }).waitFor({ timeout: 15000 });
  }

  // 4c. The open conversation is read again only when its own entry
  //     moved: another session's rename redraws the list alone.
  let reads = 0;
  page.on('request', (r) => { if (new URL(r.url()).pathname.endsWith('/conversation')) reads += 1; });
  {
    await page.waitForTimeout(1000);
    const before = reads;
    await step('rename-other');
    await until(async () => (await text(page, '#sessions')).includes('helper 2'), 'the list shows the rename');
    await page.waitForTimeout(1000);
    assert.equal(reads, before, 'another session moving reads nothing');
    await step('rename-open');
    await page.locator('#session-name', { hasText: 'investigator 5' }).waitFor({ timeout: 15000 });
    await until(() => reads === before + 1, 'one read for the open session moving');
    await page.waitForTimeout(1000);
    assert.equal(reads, before + 1, 'one read, not more');
  }

  // 4d. A turn's boundary is read once more a moment later: the
  //     transcript changes after the read the Stop caused, and nothing
  //     else moves.
  {
    const before = reads;
    await step('stop');
    await until(() => reads === before + 1, 'the read the Stop caused');
    await step('transcript-5');
    await until(async () => (await turnNumbers(page)).length === 5, 'the confirming re-read drew 5 turns', 6000);
  }

  // 4e. Back before the confirming re-read cancels it; reopening the
  //     session reads again instead of trusting the read the Stop caused.
  {
    await page.waitForTimeout(1100);
    const before = reads;
    await step('stop');
    await until(() => reads === before + 1, 'the read the second Stop caused');
    await step('transcript-6');
    await page.click('#back');
    await page.locator('#sessions .row', { hasText: 'investigator' }).click();
    await until(async () => (await turnNumbers(page)).length === 6, 'the reopened session drew 6 turns', 6000);
  }

  // 5. Tab switches never start a second polling chain. Without a
  //    socket the page polls every 3 s.
  {
    await step('one-chain');
    const second = await ctx.newPage();
    await second.addInitScript(() => { delete window.WebSocket; });
    await second.goto(new URL('/', url).href);
    await second.locator('#sessions .row').first().waitFor({ timeout: 15000 });
    for (let i = 0; i < 5; i += 1) {
      await second.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
    }
    await second.waitForTimeout(2000);
    let waiting = 0;
    second.on('request', (r) => { if (new URL(r.url()).pathname === '/api/v1/waiting') waiting += 1; });
    await second.waitForTimeout(9000);
    console.log(`polls in 9 s: ${waiting}`);
    assert.ok(waiting <= 4, `${waiting} polls in 9 s`);
    await second.close();
  }

  // 6. A send reaches the pane once and says so.
  await step('send');
  await page.fill('#reply-text', 'second');
  let release;
  const held = new Promise((resolve) => { release = resolve; });
  let reached;
  const seen = new Promise((resolve) => { reached = resolve; });
  await page.route('**/api/v1/sessions/*/send', async (route) => {
    reached();
    await held;
    await route.continue();
  });
  await page.click('#send');
  await seen;
  await page.locator('#reply-text').pressSequentially(' extra');
  release();
  await page.locator('#reply-status', { hasText: 'Sent.' }).waitFor({ timeout: 15000 });
  assert.equal(await text(page, '#reply-status'), 'Sent.');
  assert.equal(await page.inputValue('#reply-text'), ' extra');
  await page.unroute('**/api/v1/sessions/*/send');
  await step('sent');

  // 6a. A finished command lists its runs, newest chosen, with how each
  //     ended and the files it made, and shows the chosen run's output
  //     in place of "not running"; another run is a click away.
  await step('finished-command');
  await page.click('#back');
  await page.click('#filters-toggle');
  await page.selectOption('#f-state', 'all');
  await page.fill('#search', 'build');
  await page.locator('#sessions .row', { hasText: 'finished build' }).click();
  await page.locator('#session-text', { hasText: 'second run ok' }).waitFor({ timeout: 15000 });
  assert.equal(await page.locator('#run-list .run').count(), 2);
  const runs = await text(page, '#run-list');
  for (const word of ['Run 1', 'Run 2', 'exit 2', 'ok', 'report.txt']) assert.ok(runs.includes(word), `${word} in ${runs}`);
  assert.ok(!(await text(page, '#session-text')).includes('not running'));
  assert.ok((await text(page, '#run-list .run[aria-current="true"]')).startsWith('Run 2'), 'the newest run is chosen');
  await page.locator('#run-list .run', { hasText: 'Run 1' }).click();
  await page.locator('#session-text', { hasText: 'first run failed' }).waitFor({ timeout: 15000 });
  await fitsWidth(page);

  // 6b. While a command's pane runs, the block keeps its screen, and
  //     the open run is listed beneath it; a poll tick leaves the
  //     screen alone.
  await step('running-command');
  await page.click('#back');
  await page.locator('#sessions .row', { hasText: 'live build' }).click();
  await page.locator('#session-text', { hasText: 'live screen' }).waitFor({ timeout: 15000 });
  await page.locator('#run-list', { hasText: 'running' }).waitFor({ timeout: 15000 });
  await page.waitForTimeout(3500);
  const screen = await text(page, '#session-text');
  assert.ok(screen.includes('live screen') && !screen.includes('log text'), screen);
  await fitsWidth(page);

  // 7. A 401 from a host under another identity says so; from the
  //    same host, that the device was removed.
  const real = await page.evaluate(() => localStorage.getItem('switchboard-host'));
  await page.evaluate(() => {
    const saved = JSON.parse(localStorage.getItem('switchboard-host'));
    saved.server_id = '00000000-0000-0000-0000-000000000000';
    localStorage.setItem('switchboard-host', JSON.stringify(saved));
  });
  await step('changed-and-removed');
  await page.reload();
  await noticeTitle(page, 'Identity changed');
  await page.evaluate((value) => localStorage.setItem('switchboard-host', value), real);
  await page.reload();
  await noticeTitle(page, 'This device was removed on the host');

  console.log('DONE');
} catch (error) {
  await new Promise((resolve) => process.stderr.write(`FAIL ${current}: ${error.stack || error}\n`, resolve));
  process.exitCode = 1;
} finally {
  await browser.close();
  process.stdin.destroy();
}
