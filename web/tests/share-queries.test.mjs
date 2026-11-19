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
const { keepAnswer, rereadLinks, sentAt, shareQuery, sharesQuery } = await import('../src/api.ts');
const { LinkClock, forget, leaving } = await import('../src/link-clock.svelte.ts');

/** What the share dialog takes as a change goes out. */
const sending = () => ({ sent: sentAt(), since: leaving() });

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
  answer(stopped);
  await client.fetchQuery(shareQuery('cedar'));
  const sent = sentAt();
  // Stop sharing went out; another tab shared again, and this page read the new link.
  const replacement = link();
  answer(replacement);
  await client.fetchQuery({ ...shareQuery('cedar'), staleTime: 0 });
  await keepAnswer(client, 'cedar', null, sent);
  assert.deepEqual(client.getQueryData(['share', 'cedar']), replacement);
});

test("a change's answer lands when nothing newer was read meanwhile", async () => {
  const client = new QueryClient();
  const before = link();
  answer(before);
  await client.fetchQuery(shareQuery('cedar'));
  const sent = sentAt();
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
  const sent = sentAt();
  // Sharing went out; Settings then read the list (another tab had stopped and replaced it).
  answer({ shares: [link()] });
  await client.fetchQuery(sharesQuery);
  const made = link();
  await keepAnswer(client, 'cedar', made, sent);
  assert.notDeepEqual(client.getQueryData(['share', 'cedar']), made);
});

test('a read landing in the same millisecond as the last one still counts as newer', async () => {
  const client = new QueryClient();
  const now = Date.now;
  Date.now = () => 1_800_000_000_000;
  try {
    answer(null);
    await client.fetchQuery(shareQuery('cedar'));
    const sent = sentAt();
    const replacement = link();
    answer(replacement);
    await client.fetchQuery({ ...shareQuery('cedar'), staleTime: 0 });
    await keepAnswer(client, 'cedar', null, sent);
    assert.deepEqual(client.getQueryData(['share', 'cedar']), replacement);
  } finally {
    Date.now = now;
  }
});

test("a read counts as newer even after Settings' list was dropped from memory and read afresh", async () => {
  const client = new QueryClient();
  answer({ shares: [] });
  await client.fetchQuery(sharesQuery);
  const sent = sentAt();
  // Unused for a while, the list is dropped; reopening Settings reads it again from scratch.
  client.removeQueries({ queryKey: ['shares'] });
  answer({ shares: [] });
  await client.fetchQuery(sharesQuery);
  const stale = link();
  await keepAnswer(client, 'cedar', stale, sent);
  assert.notDeepEqual(client.getQueryData(['share', 'cedar']), stale);
});

test('a link any held copy shows as lapsed is not taken for gone', async () => {
  const client = new QueryClient();
  const url = link().url;
  // Settings still holds the link as never expiring; the recording page has read its deadline.
  client.setQueryData(['shares'], { shares: [{ ...link(), url, expires_at: null }] });
  const lapsedCopy = { ...link(), url, expires_at: new Date(Date.now() - 1000).toISOString() };
  client.setQueryData(['share', 'cedar'], lapsedCopy);
  // Read at the deadline, before an extension committed: no link. Then the extension.
  answer(null);
  await client.fetchQuery(shareQuery('cedar'));
  const extended = { ...lapsedCopy, expires_at: new Date(Date.now() + 86_400_000).toISOString() };
  answer(extended);
  await client.fetchQuery({ ...shareQuery('cedar'), staleTime: 0 });
  assert.equal(offered(extended), true);
});

test('a link a racing read took for gone is offered again once a later read returns it', async () => {
  const client = new QueryClient();
  // This tab holds the link as never expiring; another tab set a deadline, and an extension made
  // at that deadline was still committing when this tab read: no link.
  const held = link();
  client.setQueryData(['share', 'cedar'], held);
  answer(null);
  await client.fetchQuery(shareQuery('cedar'));
  assert.equal(offered(held), false);
  // A later read finds the same link working: it was never gone.
  answer(held);
  await client.fetchQuery({ ...shareQuery('cedar'), staleTime: 0 });
  assert.equal(offered(held), true);
});

test('a read that left before a link was stopped does not bring it back', async () => {
  const client = new QueryClient();
  const stopped = link();
  client.setQueryData(['share', 'cedar'], stopped);
  const release = answerLater(stopped);
  const read = client.fetchQuery(shareQuery('cedar'));
  await delay(0);
  // The owner stops the link while that read is on its way.
  forget(stopped.url);
  release();
  await read;
  assert.equal(offered(stopped), false);
});

test("an older change's answer arriving after a newer one does not overwrite it", async () => {
  const client = new QueryClient();
  const shared = link();
  answer(shared);
  await client.fetchQuery(shareQuery('cedar'));
  const first = sentAt(); // Never expires, answered late.
  const second = sentAt(); // One day, answered first.
  const oneDay = { ...shared, expires_at: new Date(Date.now() + 86_400_000).toISOString() };
  await keepAnswer(client, 'cedar', oneDay, second);
  await keepAnswer(client, 'cedar', { ...shared, expires_at: null }, first);
  assert.deepEqual(client.getQueryData(['share', 'cedar']), oneDay);
});

test('sharing again brings back a working link a racing read took for gone', async () => {
  const client = new QueryClient();
  const held = link();
  client.setQueryData(['share', 'cedar'], held);
  answer(null); // A read racing an extension at the deadline: no link.
  await client.fetchQuery(shareQuery('cedar'));
  assert.equal(offered(held), false);
  // The owner chooses Anyone with the link; the server answers with the same working link.
  const { sent, since } = sending();
  await keepAnswer(client, 'cedar', held, sent, since);
  assert.equal(offered(held), true);
});

test('an answer does not bring back a link stopped after the change was sent', async () => {
  const client = new QueryClient();
  const shared = link();
  client.setQueryData(['share', 'cedar'], shared);
  const { sent, since } = sending();
  forget(shared.url); // Stopped (elsewhere on this page) while the change was on its way.
  await keepAnswer(client, 'cedar', shared, sent, since);
  assert.equal(offered(shared), false);
});

test('an older read missing a link does not mark it gone after a newer read returned it', async () => {
  const client = new QueryClient();
  const held = link();
  client.setQueryData(['share', 'cedar'], held);
  // The recording's read leaves, and is slow; it raced a change and will answer no link.
  const release = answerLater(null);
  const older = client.fetchQuery(shareQuery('cedar'));
  await delay(0);
  // Settings reads after it and gets the link back working.
  answer({ shares: [held] });
  await client.fetchQuery(sharesQuery);
  // The older read lands last.
  release();
  await older;
  assert.equal(offered(held), true);
});

test("a change's answer is shown at once and then read again, so crossed answers settle", async () => {
  const client = new QueryClient();
  const shared = link();
  answer(shared);
  await client.fetchQuery(shareQuery('cedar'));
  const { sent, since } = sending();
  const sevenDays = { ...shared, expires_at: new Date(Date.now() + 7 * 86_400_000).toISOString() };
  await keepAnswer(client, 'cedar', sevenDays, sent, since);
  assert.deepEqual(client.getQueryData(['share', 'cedar']), sevenDays);
  // Another change may have committed after this one: the next read settles it.
  assert.equal(client.getQueryState(['share', 'cedar']).isInvalidated, true);
});

test('a newer read returning a link undoes an older omission that landed first', async () => {
  const client = new QueryClient();
  const held = link();
  client.setQueryData(['share', 'cedar'], held);
  // The recording's read leaves first and will answer no link; Settings' read leaves after it
  // and will answer the link working. The older one lands first.
  let releaseOlder, releaseNewer;
  globalThis.fetch = (url) =>
    new Promise((resolve) => {
      const body = String(url).endsWith('/v1/shares') ? { shares: [held] } : null;
      const go = () => resolve(Response.json(body));
      if (body) releaseNewer = go;
      else releaseOlder = go;
    });
  const older = client.fetchQuery(shareQuery('cedar'));
  await delay(0);
  const newer = client.fetchQuery(sharesQuery);
  await delay(0);
  releaseOlder();
  await older;
  releaseNewer();
  await newer;
  assert.equal(offered(held), true);
});

test('an answer that is not shown still counts, so a later one cannot overwrite what it settled', async () => {
  const client = new QueryClient();
  const shared = link();
  answer(shared);
  await client.fetchQuery(shareQuery('cedar'));
  const first = sending(); // One day.
  answer(shared);
  await client.fetchQuery({ ...shareQuery('cedar'), staleTime: 0 }); // A read lands meanwhile.
  const second = sending(); // Seven days, committed first at the server.
  const oneDay = { ...shared, expires_at: new Date(Date.now() + 86_400_000).toISOString() };
  const sevenDays = { ...shared, expires_at: new Date(Date.now() + 7 * 86_400_000).toISOString() };
  // The one-day answer arrives first; a read landed since it was sent, so it is not shown.
  await keepAnswer(client, 'cedar', oneDay, first.sent, first.since);
  // The seven-day answer arrives after it: an answer landed since it was sent, so it is not shown
  // either, and the read after both settles it.
  await keepAnswer(client, 'cedar', sevenDays, second.sent, second.since);
  assert.notDeepEqual(client.getQueryData(['share', 'cedar']), sevenDays);
});

test('after a change, Settings reads its list afresh even if its first read was still on its way', async () => {
  const client = new QueryClient();
  let listReads = 0;
  let releaseFirst;
  globalThis.fetch = (url) => {
    if (!String(url).endsWith('/v1/shares')) return Promise.resolve(Response.json(null));
    listReads++;
    if (listReads === 1)
      return new Promise(
        (resolve) => (releaseFirst = () => resolve(Response.json({ shares: [] }))),
      );
    return Promise.resolve(Response.json({ shares: [link()] }));
  };
  // Settings opens for the first time while sharing is on its way: its list read leaves.
  const { QueryObserver } = await import('@tanstack/query-core');
  const settings = new QueryObserver(client, sharesQuery);
  const stop = settings.subscribe(() => {});
  await delay(0);
  const { sent, since } = sending();
  const kept = keepAnswer(client, 'cedar', link(), sent, since);
  // The first read lands after the change answered: it left before the change.
  releaseFirst();
  await kept;
  await delay(10);
  stop();
  client.clear();
  assert.equal(listReads, 2);
});

test("a change that failed still counts, so an earlier change's answer arriving after it is not shown", async () => {
  const client = new QueryClient();
  const shared = link();
  answer(shared);
  await client.fetchQuery(shareQuery('cedar'));
  const first = sending(); // Seven days, answered late.
  // A one-day change committed but its answer was lost: the page asks again.
  await rereadLinks(client, 'cedar');
  const sevenDays = { ...shared, expires_at: new Date(Date.now() + 7 * 86_400_000).toISOString() };
  await keepAnswer(client, 'cedar', sevenDays, first.sent, first.since);
  assert.notDeepEqual(client.getQueryData(['share', 'cedar']), sevenDays);
});

test("after a change, Settings' list is read again even while Settings is not shown", async () => {
  const client = new QueryClient();
  const before = link();
  answer({ shares: [before] });
  await client.fetchQuery(sharesQuery);
  let listReads = 0;
  const changed = { ...before, expires_at: new Date(Date.now() + 86_400_000).toISOString() };
  globalThis.fetch = async (url) => {
    if (!String(url).endsWith('/v1/shares')) return Response.json(changed);
    listReads++;
    return Response.json({ shares: [changed] });
  };
  const { sent, since } = sending();
  await keepAnswer(client, 'cedar', changed, sent, since);
  assert.equal(listReads, 1);
  assert.deepEqual(client.getQueryData(['shares']), { shares: [changed] });
});
