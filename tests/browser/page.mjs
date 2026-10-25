// The browser half of `the_page_works_in_a_real_browser` in
// tests/remote.rs: drives the web page in headless Chromium against the
// test's host. Before each step it prints `STEP <name>` and waits for a
// line on stdin, the host's answer once it has done that step's part:
// `go`, or `invite <url>` for a pairing window it opened. The host is
// served only while the Rust side pumps it, so this side never runs
// ahead of an answer. The first failed check prints `FAIL <step>:` and
// exits 1; a full run ends with `DONE`. The `midflight text:`,
// `socket:` and `polls in 9 s:` lines are printed measurements, not
// checks.
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
  console.log(`midflight text: ${(await page.inputValue('#reply-text')).includes(' extra') ? 'kept' : 'cleared'}`);
  await page.unroute('**/api/v1/sessions/*/send');
  await step('sent');

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
