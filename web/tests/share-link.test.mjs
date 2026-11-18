import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { registerHooks, stripTypeScriptTypes } from 'node:module';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { compileModule } from 'svelte/compiler';
// Svelte's client runtime, as the compiled module uses it (see playback.test.mjs).
import { effect_root, flush } from 'svelte/internal/client';

// A `.svelte.ts` module uses runes, so it is compiled as Vite would before it is imported.
registerHooks({
  load(url, context, next) {
    if (!url.endsWith('.svelte.ts')) return next(url, context);
    const source = stripTypeScriptTypes(readFileSync(new URL(url), 'utf8'));
    const { js } = compileModule(source, { filename: url, generate: 'client' });
    return { format: 'module', source: js.code, shortCircuit: true };
  },
});
// The page listens for being shown again; Node has no document.
globalThis.document ??= new EventTarget();
const { LinkClock } = await import('../src/share-link.svelte.ts');

const expiring = (ms) => {
  const expires_at = ms === null ? null : new Date(Date.now() + ms).toISOString();
  return {
    url: 'https://vault.example.com/s/link',
    created_at: '2026-11-18T10:00:00.000Z',
    expires_at,
  };
};
/** The clock of a page showing `links`, as the recording page, the share dialog and Settings
 * build it, and how often it asked the server again. */
function page(...links) {
  let clock;
  const asked = { count: 0 };
  const stop = effect_root(() => {
    clock = new LinkClock(
      () => links.map((link) => link.expires_at),
      () => asked.count++,
    );
  });
  flush();
  return { clock, asked, stop };
}

test('a link stops being offered when it expires, without waiting for the server', async () => {
  const share = expiring(200);
  const { clock, stop } = page(share);
  try {
    assert.equal(clock.offered(share), share);
    await delay(300);
    assert.equal(clock.offered(share), null);
  } finally {
    stop();
  }
});

test('a page whose timer runs late checks the clock before it offers the link', () => {
  const share = expiring(60_000);
  const { clock, stop } = page(share);
  const now = Date.now;
  try {
    assert.equal(clock.offered(share), share);
    // A minute on, before the timer has run, as in a page that was hidden meanwhile.
    Date.now = () => now() + 61_000;
    clock.check();
    assert.equal(clock.offered(share), null);
  } finally {
    Date.now = now;
    stop();
  }
});

test('a page checks the clock when it is shown again', () => {
  const share = expiring(60_000);
  const { clock, stop } = page(share);
  const now = Date.now;
  try {
    Date.now = () => now() + 61_000;
    document.dispatchEvent(new Event('visibilitychange'));
    assert.equal(clock.offered(share), null);
  } finally {
    Date.now = now;
    stop();
  }
});

test('a link that never expires stays offered', () => {
  const share = expiring(null);
  const { clock, stop } = page(share);
  try {
    clock.check();
    assert.equal(clock.offered(share), share);
  } finally {
    stop();
  }
});

test('a link checked a moment before it expires still lapses when it does', async () => {
  const share = expiring(50);
  const deadline = Date.parse(share.expires_at);
  let clock;
  const stop = effect_root(() => {
    clock = new LinkClock(
      () => [share.expires_at],
      () => {},
    );
  });
  // The page reads its clock a millisecond before the deadline, then again at it.
  const now = Date.now;
  let reads = 0;
  Date.now = () => (++reads === 1 ? deadline - 1 : reads === 2 ? deadline : now());
  try {
    flush();
  } finally {
    Date.now = now;
  }
  try {
    await delay(150);
    assert.equal(clock.offered(share), null);
  } finally {
    stop();
  }
});

test('a page asks the server again when its link lapses, once', async () => {
  const share = expiring(100);
  const { clock, asked, stop } = page(share);
  try {
    assert.equal(asked.count, 0);
    await delay(200);
    assert.equal(clock.offered(share), null);
    assert.equal(asked.count, 1);
    document.dispatchEvent(new Event('visibilitychange'));
    assert.equal(asked.count, 1);
  } finally {
    stop();
  }
});

test('a list of links drops each one as it expires and asks again each time', async () => {
  const first = expiring(100);
  const second = expiring(250);
  const lasting = expiring(null);
  const { clock, asked, stop } = page(first, second, lasting);
  try {
    await delay(170);
    assert.equal(clock.offered(first), null);
    assert.equal(clock.offered(second), second);
    assert.equal(asked.count, 1);
    await delay(150);
    assert.equal(clock.offered(second), null);
    assert.equal(clock.offered(lasting), lasting);
    assert.equal(asked.count, 2);
  } finally {
    stop();
  }
});
