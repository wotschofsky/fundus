/** Reads started as the page loads, alongside the app's own code rather than after it, so no
 * read waits for another: the sign-in check, and a recording's page and what it shows beside. */
const started = new Map<string, Promise<Response>>();

export function start(urls: string[], init?: RequestInit) {
  for (const url of urls) started.set(url, fetch(url, init));
}

/** The early read of `url`, handed out once, whatever it answered (asking again would only wait for
 * the same answer); one that never arrived is not, and its reader asks afresh. */
// ponytail: an early read taken late (the vault's sign-in check failed and was tried again) is
// as old as the page; the reader's next refresh brings it up to date.
export async function take(url: string): Promise<Response | undefined> {
  const read = started.get(url);
  started.delete(url);
  return await read?.catch(() => undefined);
}

/** Signing in makes every read started before it stale: each was made signed out. */
export function dropAll() {
  started.clear();
}
