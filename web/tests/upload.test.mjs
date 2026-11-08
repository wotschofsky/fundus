import assert from 'node:assert/strict';
import test from 'node:test';
import { upload, verified } from '../src/upload.ts';

// A tus server in memory. The second chunk of the large file drops mid-way, after the server
// kept half of it, so the client has to ask HEAD where to continue.
function vault() {
  const uploads = new Map();
  const state = { completed: false, polls: 0, dropped: false, names: [] };
  const reply = (status, headers = {}, body = null) =>
    new Response(body && JSON.stringify(body), { status, headers });
  globalThis.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const headers = new Headers(init.headers);
    if (method === 'POST' && url === '/api/web/v1/imports') return reply(200, {}, { id: 'import' });
    if (method === 'POST' && url === '/api/web/v1/uploads') {
      const meta = Object.fromEntries(
        headers
          .get('Upload-Metadata')
          .split(',')
          .map((item) => item.split(' '))
          .map(([key, value]) => [key, Buffer.from(value, 'base64').toString()]),
      );
      assert.equal(meta.import_id, 'import');
      state.names.push(meta.filename);
      const location = `/api/web/v1/uploads/${uploads.size}`;
      const length = Number(headers.get('Upload-Length'));
      uploads.set(location, { bytes: new Uint8Array(length), offset: 0 });
      return reply(201, { Location: location });
    }
    if (method === 'HEAD') return reply(200, { 'Upload-Offset': String(uploads.get(url).offset) });
    if (method === 'POST' && url === '/api/web/v1/imports/import/complete') {
      state.completed = true;
      return reply(200, {}, { state: 'verifying' });
    }
    if (method === 'GET' && url === '/api/web/v1/imports/import') {
      const files = [{ filename: 'memo ü.m4a', state: 'duplicate', failure: null }];
      return reply(200, {}, { state: state.polls++ ? 'complete' : 'verifying', files });
    }
    return reply(404, {}, { error: 'Not found' });
  };
  globalThis.XMLHttpRequest = class {
    upload = {};
    #headers = {};
    #response = {};
    open(method, url) {
      assert.equal(method, 'PATCH');
      this.url = url;
    }
    setRequestHeader(key, value) {
      this.#headers[key] = value;
    }
    getResponseHeader(key) {
      return this.#response[key];
    }
    async send(chunk) {
      const target = uploads.get(this.url);
      const bytes = new Uint8Array(await chunk.arrayBuffer());
      if (Number(this.#headers['Upload-Offset']) !== target.offset) {
        this.status = 409;
        this.responseText = JSON.stringify({ error: 'Offset mismatch' });
        return this.onload();
      }
      if (target.offset > 0 && !state.dropped) {
        state.dropped = true;
        const kept = bytes.subarray(0, bytes.length >> 1);
        target.bytes.set(kept, target.offset);
        target.offset += kept.length;
        this.upload.onprogress({ loaded: kept.length });
        return this.onerror();
      }
      target.bytes.set(bytes, target.offset);
      target.offset += bytes.length;
      this.upload.onprogress({ loaded: bytes.length });
      this.status = 204;
      this.#response['Upload-Offset'] = String(target.offset);
      this.onload();
    }
  };
  return { uploads, state };
}

test('uploads in chunks, resumes where the server stopped, and reports the check', async () => {
  const { uploads, state } = vault();
  const large = new Uint8Array(9 * 1024 * 1024 + 3).map((_, i) => i % 251);
  const files = [new File([large], 'long.wav'), new File(['hello'], 'memo ü.m4a')];
  const total = large.length + 5;
  const seen = [];
  const id = await upload(files, (sent, index) => seen.push([sent, index]));

  assert.equal(id, 'import');
  assert.deepEqual(state.names, ['long.wav', 'memo ü.m4a']);
  assert.ok(state.dropped, 'the dropped chunk was exercised');
  assert.deepEqual(uploads.get('/api/web/v1/uploads/0').bytes, large);
  assert.equal(new TextDecoder().decode(uploads.get('/api/web/v1/uploads/1').bytes), 'hello');
  assert.ok(state.completed);
  // Progress never runs backwards, and ends on the whole batch.
  for (let i = 1; i < seen.length; i++) assert.ok(seen[i][0] >= seen[i - 1][0], `${seen}`);
  assert.deepEqual(seen.at(-1), [total, 1]);

  const result = await verified(id);
  assert.equal(result.state, 'complete');
  assert.equal(result.files[0].state, 'duplicate');
});

test('a refused upload surfaces the server message without retrying', async () => {
  vault();
  const fetch = globalThis.fetch;
  globalThis.fetch = async (url, init) =>
    url === '/api/web/v1/uploads'
      ? new Response(JSON.stringify({ error: 'Invalid filename' }), { status: 400 })
      : fetch(url, init);
  await assert.rejects(
    upload([new File(['x'], 'x.wav')], () => {}),
    /Invalid filename/,
  );
});
