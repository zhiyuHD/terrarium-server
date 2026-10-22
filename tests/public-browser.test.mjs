import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, test } from 'node:test';
import { runInNewContext } from 'node:vm';

for (const page of ['forum', 'index']) describe(page, () => {
const html = readFileSync(new URL(`../web/${page}.html`, import.meta.url), 'utf8');
const start = html.indexOf('async function fetchJson(');
const source = html.slice(start, page === 'forum' ? html.indexOf('async function api(') : html.indexOf('</script>', start));
function browser(fetch) {
  const timers = new Map();
  const read = runInNewContext(`${source}\nfetchJson`, {
    AbortController, AbortSignal: {}, fetch,
    setTimeout(callback, delay) { const id = {}; timers.set(id, { callback, delay }); return id; },
    clearTimeout(id) { timers.delete(id); },
  });
  return { read, timers };
}
function cancelled(signal) {
  return new Promise((resolve, reject) => {
    const abort = () => reject(new DOMException('Aborted', 'AbortError'));
    if (signal.aborted) abort(); else signal.addEventListener('abort', abort, { once: true });
  });
}
function navigation() {
  const controller = new AbortController(), listeners = new Set();
  const signal = {
    get aborted() { return controller.signal.aborted; },
    addEventListener(type, listener, options) { listeners.add(listener); controller.signal.addEventListener(type, listener, options); },
    removeEventListener(type, listener) { listeners.delete(listener); controller.signal.removeEventListener(type, listener); },
  };
  return { controller, signal, listeners };
}

test('JSON reads work without AbortSignal static helpers and release their timer and navigation listener', async () => {
  const nav = navigation(), response = new Response('{"enabled":true}');
  const { read, timers } = browser(async () => response);
  const result = await read('/api/forum/status', nav.signal);
  assert.equal(result.response, response);
  assert.equal(result.value.enabled, true);
  assert.equal(timers.size, 0); assert.equal(nav.listeners.size, 0);
});

test('changing pages cancels a pending request and releases resources', async () => {
  const nav = navigation();
  const { read, timers } = browser(async (_, { signal }) => cancelled(signal));
  const pending = read('/api/forum/threads', nav.signal);
  nav.controller.abort();
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(timers.size, 0); assert.equal(nav.listeners.size, 0);
});

test('a page already left cannot issue an uncancelled request', async () => {
  const nav = navigation(); nav.controller.abort();
  const { read, timers } = browser(async (_, { signal }) => { assert.equal(signal.aborted, true); return cancelled(signal); });
  await assert.rejects(read('/api/forum/status', nav.signal), { name: 'AbortError' });
  assert.equal(timers.size, 0); assert.equal(nav.listeners.size, 0);
});

test('the timeout also aborts a scene response stalled while reading JSON', async () => {
  const nav = navigation(); let bodyStarted;
  const started = new Promise(resolve => { bodyStarted = resolve; });
  const { read, timers } = browser(async (_, { signal }) => ({ async json() { bodyStarted(); return cancelled(signal); } }));
  const pending = read('/api/boxes/fixture-box?view=scene', nav.signal);
  await started;
  const timer = [...timers.values()][0]; assert.equal(timer.delay, 15_000);
  timer.callback();
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(timers.size, 0); assert.equal(nav.listeners.size, 0);
});

test('HTTP error payloads remain available and invalid JSON releases resources', async () => {
  const nav = navigation();
  const { read, timers } = browser(async url => new Response(url === '/missing' ? '{"error":"Missing"}' : '{', { status: 404 }));
  const result = await read('/missing', nav.signal);
  assert.equal(result.response.status, 404); assert.equal(result.value.error, 'Missing');
  await assert.rejects(read('/invalid', nav.signal), SyntaxError);
  assert.equal(timers.size, 0); assert.equal(nav.listeners.size, 0);
});

if (page === 'index') test('feedback keeps POST options and a ten-second deadline without a navigation signal', async () => {
  let complete;
  const { read, timers } = browser((url, init) => {
    assert.equal(url, '/api/feedback'); assert.equal(init.method, 'POST');
    assert.equal(init.body, '{"text":"Test feedback"}'); assert.equal(init.cache, 'no-store');
    assert.equal(init.headers['content-type'], 'application/json');
    return new Promise(resolve => { complete = () => resolve(new Response('{"id":1}')); });
  });
  const pending = read('/api/feedback', undefined, { method: 'POST', headers: {'content-type':'application/json'}, body: '{"text":"Test feedback"}', cache: 'no-store', timeoutMs: 10_000 });
  assert.equal([...timers.values()][0].delay, 10_000);
  complete(); assert.equal((await pending).value.id, 1); assert.equal(timers.size, 0);
});
});
