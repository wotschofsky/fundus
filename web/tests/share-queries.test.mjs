import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { registerHooks, stripTypeScriptTypes } from 'node:module';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { compileModule } from 'svelte/compiler';
import { effect_root, flush } from 'svelte/internal/client';
import ts from 'typescript';
import { QueryClient } from '@tanstack/query-core';

// api.ts takes only `queryOptions` from the Svelte adapter, whose components Node cannot load,
// and declares class fields in a constructor, which Node's type stripping refuses; the page clock
// is a `.svelte.ts` module, compiled as Vite would. Imports leave out `.ts` as Vite allows.
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === '@tanstack/svelte-query')
      return { url: 'test:svelte-query', shortCircuit: true };
    try {
      return next(specifier, context);
    } catch (error) {
      if (!specifier.startsWith('.')) throw error;
      return next(`${specifier}.ts`, context);
    }
  },
  load(url, context, next) {
    if (url === 'test:svelte-query') {
      const source = 'export const queryOptions = (options) => options;';
      return { format: 'module', source, shortCircuit: true };
    }
    if (url.endsWith('.svelte.ts')) {
      const source = stripTypeScriptTypes(readFileSync(new URL(url), 'utf8'));
      const { js } = compileModule(source, { filename: url, generate: 'client' });
      return { format: 'module', source: js.code, shortCircuit: true };
    }
    if (!url.endsWith('/src/api.ts')) return next(url, context);
    const { outputText } = ts.transpileModule(readFileSync(new URL(url), 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
    });
    return { format: 'module', source: outputText, shortCircuit: true };
  },
});
globalThis.document ??= new EventTarget();
const { keepAnswer, sentAt, shareQuery, sharesQuery } = await import('../src/api.ts');
const { LinkClock } = await import('../src/link-clock.svelte.ts');

let made = 0;
/** A link that never expires, with its own URL. */
const link = (entry_id = 'cedar') => ({
  url: `https://vault.example.com/s/link${++made}`,
  created_at: '2026-11-18T10:00:00.000Z',
  expires_at: null,
  entry_id,
  title: 'Cedar launch',
});
/** Whether the owner's pages would still offer `share`. */
function offered(share) {
  let clock;
  const stop = effect_root(() => {
    clock = new LinkClock(
      () => [],
      () => {},
    );
  });
  flush();
  const result = clock.offered(share);
  stop();
  return result === share;
}
/** The server answers the next request with `body`, once the test lets it go. */
function answerLater(body) {
  let release;
  globalThis.fetch = () => new Promise((resolve) => (release = () => resolve(Response.json(body))));
  return () => release();
}
const answer = (body) => (globalThis.fetch = async () => Response.json(body));

test('a read that answers no link after the owner shared does not hide the new link', async () => {
  const client = new QueryClient();
  client.setQueryData(['share', 'cedar'], null);
  const release = answerLater(null);
  const read = client.fetchQuery(shareQuery('cedar'));
  await delay(0);
  // Sharing answered meanwhile, with the link it made.
  const fresh = link();
  client.setQueryData(['share', 'cedar'], fresh);
  release();
  await read;
  assert.equal(offered(fresh), true);
});

test("Settings' list leaving out a link the recording page had cached makes it gone", async () => {
  const client = new QueryClient();
  const stopped = link();
  client.setQueryData(['share', 'cedar'], stopped);
  answer({ shares: [] });
  await client.fetchQuery(sharesQuery);
  assert.equal(offered(stopped), false);
});

test("a recording's read leaving out a link Settings had listed makes it gone", async () => {
  const client = new QueryClient();
  const stopped = link();
  const other = link('other');
  client.setQueryData(['shares'], { shares: [stopped, other] });
  answer(null);
  await client.fetchQuery(shareQuery('cedar'));
  assert.equal(offered(stopped), false);
  assert.equal(offered(other), true);
});

test('a link made while Settings was reading stays offered', async () => {
  const client = new QueryClient();
  const release = answerLater({ shares: [] });
  const read = client.fetchQuery(sharesQuery);
  await delay(0);
  const fresh = link();
  client.setQueryData(['share', 'cedar'], fresh);
  release();
  await read;
  assert.equal(offered(fresh), true);
});

test("a change's answer does not overwrite a read that landed after the change was sent", async () => {
  const client = new QueryClient();
  const stopped = link();
  client.setQueryData(['share', 'cedar'], stopped);
  const sent = sentAt(client, 'cedar');
  await delay(2);
  // Stop sharing went out; another tab shared again, and this page read the new link.
  const replacement = link();
  client.setQueryData(['share', 'cedar'], replacement);
  await keepAnswer(client, 'cedar', null, sent);
  assert.deepEqual(client.getQueryData(['share', 'cedar']), replacement);
});

test("a change's answer lands when nothing newer was read meanwhile", async () => {
  const client = new QueryClient();
  const before = link();
  client.setQueryData(['share', 'cedar'], before);
  const sent = sentAt(client, 'cedar');
  const changed = { ...before, expires_at: '2026-12-01T00:00:00.000Z' };
  await keepAnswer(client, 'cedar', changed, sent);
  assert.deepEqual(client.getQueryData(['share', 'cedar']), changed);
});

test('a link that only lapsed is not taken for gone, so an extension made at its deadline shows', async () => {
  const client = new QueryClient();
  const lapsed = { ...link(), expires_at: new Date(Date.now() - 1000).toISOString() };
  client.setQueryData(['share', 'cedar'], lapsed);
  // Read at the deadline from before the extension committed: no link.
  answer(null);
  await client.fetchQuery(shareQuery('cedar'));
  // Read again: the same link, extended.
  const extended = { ...lapsed, expires_at: new Date(Date.now() + 86_400_000).toISOString() };
  answer(extended);
  await client.fetchQuery({ ...shareQuery('cedar'), staleTime: 0 });
  assert.equal(offered(extended), true);
});

test("a change's answer does not overwrite what Settings read after the change was sent", async () => {
  const client = new QueryClient();
  client.setQueryData(['share', 'cedar'], null);
  const sent = sentAt(client, 'cedar');
  // Sharing went out; Settings then read the list (another tab had stopped and replaced it).
  const replacement = link();
  client.setQueryData(['shares'], { shares: [replacement] });
  const made = link();
  await keepAnswer(client, 'cedar', made, sent);
  assert.notDeepEqual(client.getQueryData(['share', 'cedar']), made);
});

test('a read landing in the same millisecond as the last one still counts as newer', async () => {
  const client = new QueryClient();
  const now = Date.now;
  Date.now = () => 1_800_000_000_000;
  try {
    client.setQueryData(['share', 'cedar'], null);
    const sent = sentAt(client, 'cedar');
    const replacement = link();
    client.setQueryData(['share', 'cedar'], replacement);
    await keepAnswer(client, 'cedar', null, sent);
    assert.deepEqual(client.getQueryData(['share', 'cedar']), replacement);
  } finally {
    Date.now = now;
  }
});
