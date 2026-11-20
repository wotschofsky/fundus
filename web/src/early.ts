/** Reads started as the page loads, alongside the app's own code rather than after it, so no
 * read waits for another: the sign-in check, and a recording's page and what it shows beside. */
const started = new Map<string, Promise<Response>>();

export function start(urls: string[], init?: RequestInit) {
  for (const url of urls) started.set(url, fetch(url, init));
}

/** The early read of `url`, handed out once, whatever it answered (asking again would only wait for
 * the same answer); one that never arrived is not, and its reader asks afresh. */
export async function take(url: string): Promise<Response | undefined> {
  const read = started.get(url);
  started.delete(url);
  return await read?.catch(() => undefined);
}

/** Drops every early read not yet taken: its reader asks afresh. */
export function dropAll() {
  started.clear();
}
