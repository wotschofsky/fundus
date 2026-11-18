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
const { ShareLink } = await import('../src/share-link.svelte.ts');

const expiring = (ms) => {
  const expires_at = ms === null ? null : new Date(Date.now() + ms).toISOString();
  return {
    url: 'https://vault.example.com/s/link',
    created_at: '2026-11-18T10:00:00.000Z',
    expires_at,
  };
};
/** What a page offers for `share`, as the recording page and the share dialog build it, and
 * how often it asked the server again. */
function page(share) {
  let offer;
  const asked = { count: 0 };
  const stop = effect_root(() => {
    offer = new ShareLink(
      () => share,
      () => asked.count++,
    );
  });
  flush();
  return { offer, asked, stop };
}

test('a link stops being offered when it expires, without waiting for the server', async () => {
  const share = expiring(200);
  const { offer, stop } = page(share);
  try {
    assert.equal(offer.link, share);
    await delay(300);
    assert.equal(offer.link, null);
  } finally {
    stop();
  }
});

test('a page whose timer runs late checks the clock before it offers the link', () => {
  const share = expiring(60_000);
  const { offer, stop } = page(share);
  const now = Date.now;
  try {
    assert.equal(offer.link, share);
    // A minute on, before the timer has run, as in a page that was hidden meanwhile.
    Date.now = () => now() + 61_000;
    offer.check();
    assert.equal(offer.link, null);
  } finally {
    Date.now = now;
    stop();
  }
});

test('a page checks the clock when it is shown again', () => {
  const share = expiring(60_000);
  const { offer, stop } = page(share);
  const now = Date.now;
  try {
    Date.now = () => now() + 61_000;
    document.dispatchEvent(new Event('visibilitychange'));
    assert.equal(offer.link, null);
  } finally {
    Date.now = now;
    stop();
  }
});

test('a link that never expires stays offered', () => {
  const share = expiring(null);
  const { offer, stop } = page(share);
  try {
    offer.check();
    assert.equal(offer.link, share);
  } finally {
    stop();
  }
});

test('a link checked a moment before it expires still lapses when it does', async () => {
  const share = expiring(50);
  const deadline = Date.parse(share.expires_at);
  let offer;
  const stop = effect_root(() => {
    offer = new ShareLink(
      () => share,
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
    assert.equal(offer.link, null);
  } finally {
    stop();
  }
});

test('a page asks the server again when its link lapses, once', async () => {
  const share = expiring(100);
  const { offer, asked, stop } = page(share);
  try {
    assert.equal(asked.count, 0);
    await delay(200);
    assert.equal(offer.link, null);
    assert.equal(asked.count, 1);
  } finally {
    stop();
  }
});
