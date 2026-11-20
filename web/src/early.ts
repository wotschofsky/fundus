/** Reads started as the page loads, alongside the app's own code rather than after it, so no
 * read waits for another: the sign-in check, and a recording's page and what it shows beside. */
const started = new Map<string, Promise<Response>>();

export function start(urls: string[], init?: RequestInit) {
  for (const url of urls) started.set(url, fetch(url, init));
}

/** The early read of `url`, handed out once. One that never arrived, or that wanted signing in,
 * is not: its reader asks afresh, as it would have (after signing in). Any other answer, an error
 * too, is the reader's: asking again would only wait for the same answer. */
// ponytail: an early read taken late (the vault's sign-in check failed and was tried again) is
// as old as the page; the reader's next refresh brings it up to date.
export async function take(url: string): Promise<Response | undefined> {
  const read = started.get(url);
  started.delete(url);
  const response = await read?.catch(() => undefined);
  return response?.status === 401 ? undefined : response;
}
