/** The server takes at most 8 MiB per PATCH. */
const CHUNK = 8 * 1024 * 1024;
const BASE = '/api/web/v1';
const TUS = { 'Tus-Resumable': '1.0.0' };
const LOST = 'The connection to your vault was lost.';

export type ImportFile = { filename: string; state: string; failure: string | null };

/** An Error with the HTTP status, 0 when the connection dropped, to decide on a retry. */
const fail = (status: number, message: string) => Object.assign(new Error(message), { status });

/** The server's own message, as `api()` reads it. */
function failure(status: number, text: string) {
  let message = `Upload failed (${status})`;
  try {
    message = JSON.parse(text).error ?? message;
  } catch {}
  return fail(status, message);
}

async function request(url: string, init: RequestInit = {}) {
  const r = await fetch(url, init).catch(() => {
    throw fail(0, LOST);
  });
  if (!r.ok) throw failure(r.status, await r.text());
  return r;
}

const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const base64 = (text: string) => btoa(String.fromCharCode(...new TextEncoder().encode(text)));

/** One chunk. `fetch` reports no upload progress, so this one is XHR. */
function patch(url: string, offset: number, chunk: Blob, sent: (bytes: number) => void) {
  return new Promise<number>((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('PATCH', url);
    xhr.setRequestHeader('Tus-Resumable', '1.0.0');
    xhr.setRequestHeader('Upload-Offset', String(offset));
    xhr.setRequestHeader('Content-Type', 'application/offset+octet-stream');
    xhr.upload.onprogress = (event) => sent(event.loaded);
    xhr.onload = () =>
      xhr.status === 204
        ? resolve(Number(xhr.getResponseHeader('Upload-Offset')))
        : reject(failure(xhr.status, xhr.responseText));
    xhr.onerror = () => reject(fail(0, LOST));
    xhr.send(chunk);
  });
}

async function transfer(url: string, file: File, sent: (bytes: number) => void) {
  let offset = 0;
  for (let failures = 0; offset < file.size;) {
    try {
      const chunk = file.slice(offset, offset + CHUNK);
      offset = await patch(url, offset, chunk, (n) => sent(offset + n));
      failures = 0;
    } catch (error) {
      // A dropped connection or a stale offset: ask the server what it kept and go on from there.
      const status = (error as { status?: number }).status ?? -1;
      if (!(status === 0 || status === 409 || status >= 500) || ++failures > 3) throw error;
      await delay(1000 * failures);
      const r = await request(url, { method: 'HEAD', headers: TUS }).catch(() => null);
      if (r) offset = Number(r.headers.get('Upload-Offset'));
    }
  }
}

/**
 * Sends files as one import over tus, as the native apps do, and asks the server to verify the
 * ones that arrived. A file that fails stops the batch: what was sent of it is removed, so a
 * refusal for disk space does not leave its bytes holding that space, and the files before it
 * are still checked. Resolves with the import to watch (null when no file arrived) and, on a
 * failure, the index of the first file not uploaded and why. `progress` gets the bytes sent
 * across the batch and the index of the file being sent.
 */
export async function upload(files: File[], progress: (sent: number, index: number) => void) {
  const json = { 'Content-Type': 'application/json' };
  const r = await request(`${BASE}/imports`, { method: 'POST', headers: json, body: '{}' });
  const { id } = (await r.json()) as { id: string };
  let done = 0;
  let failed: { index: number; error: Error } | null = null;
  for (const [index, file] of files.entries()) {
    progress(done, index);
    let location: string | null = null;
    try {
      const created = await request(`${BASE}/uploads`, {
        method: 'POST',
        headers: {
          ...TUS,
          'Upload-Length': String(file.size),
          'Upload-Metadata': `import_id ${base64(id)},filename ${base64(file.name)}`,
        },
      });
      location = created.headers.get('Location')!;
      await transfer(location, file, (n) => progress(done + n, index));
    } catch (error) {
      if (location) await request(location, { method: 'DELETE', headers: TUS }).catch(() => {});
      failed = { index, error: error instanceof Error ? error : new Error(String(error)) };
      break;
    }
    done += file.size;
  }
  if (failed?.index === 0) return { id: null, failed };
  await request(`${BASE}/imports/${id}/complete`, { method: 'POST' });
  return { id, failed };
}

/** Waits until the server has checked the import, then says what became of each file. */
export async function verified(id: string): Promise<{ state: string; files: ImportFile[] }> {
  for (;;) {
    const result = await (await request(`${BASE}/imports/${id}`)).json();
    if (result.state !== 'verifying') return result;
    await delay(1500);
  }
}
