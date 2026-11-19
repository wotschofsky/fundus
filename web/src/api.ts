import { queryOptions, type QueryClient } from '@tanstack/svelte-query';
import { confirm, leaving, missing } from './link-clock.svelte';

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

export async function api<T = void>(path: string, init: RequestInit = {}): Promise<T> {
  const r = await fetch(`/api${path}`, {
    ...init,
    headers: { ...(init.body ? { 'Content-Type': 'application/json' } : {}), ...init.headers },
  });
  if (!r.ok) {
    const v = await r.json().catch(() => ({ error: `Request failed (${r.status})` }));
    throw new ApiError(r.status, v.error_description ?? v.error ?? `Request failed (${r.status})`);
  }
  return r.status === 204 ? (undefined as T) : await r.json();
}

/** JSON body helper: `send('PATCH', body)`. */
export const send = (method: string, body?: unknown): RequestInit => ({
  method,
  ...(body === undefined ? {} : { body: JSON.stringify(body) }),
});

export type Stage =
  | 'queued'
  | 'preparing'
  | 'transcribing'
  | 'recognizing'
  | 'enriching'
  | 'indexing'
  | 'paused'
  | 'failed'
  | 'complete';

export type Entry = {
  id: string;
  title: string;
  recorded_at: string | null;
  duration_ms: number;
  timezone: string;
  stage: Stage;
  active_stage: Stage | null;
  pause_requested: boolean;
  failure: string | null;
  warnings: string[];
  summary: string | null;
  summary_manual?: boolean;
  created_at: string;
  trashed_at: string | null;
  labels: string[];
  audio_available: boolean;
  /** A recording made from a video file; its size is the upright picture's, known before it is ready. */
  video: {
    state: 'preparing' | 'ready' | 'failed';
    width: number;
    height: number;
    failure: string | null;
  } | null;
  speakers?: Speaker[];
  sources?: { offset_ms: number; recorded_at: string | null; filename: string }[];
};
export type Evidence = {
  source: string;
  name?: string;
  quote?: string;
  applied?: boolean;
  score?: number;
};
export type Speaker = {
  label: string;
  person_id: string | null;
  name: string | null;
  state: 'unknown' | 'automatic' | 'manual' | 'manual_unknown';
  evidence: Evidence[];
};
export type Segment = {
  id: string;
  position: number;
  speaker: string;
  speaker_name: string | null;
  start_ms: number;
  end_ms: number;
  text: string;
};
export type Page<K extends string, T> = { [key in K]: T[] } & { next_cursor: number | null };
export type Person = { id: string; name: string; samples: number };
export type Label = { name: string; count: number };
export type Me = {
  scope: string;
  auth_mode: 'proxy' | 'password';
  recorder_timezone: string;
  voice_configured: boolean;
  public_url: string;
};
export type Token = {
  id: string;
  name: string;
  scope: 'upload' | 'read';
  created_at: string;
  last_used_at: string | null;
};
export type Connection = {
  id: string;
  name: string;
  scope: string;
  created_at: number;
  last_used_at: string | null;
};
export type SearchResult = {
  id: string;
  entry_id: string;
  title: string;
  recorded_at: string | null;
  start_ms: number;
  end_ms: number;
  excerpt: string;
  summary?: boolean;
};
export type SearchResponse = {
  results: SearchResult[];
  mode: 'hybrid' | 'keyword';
  coverage: { pending_entries: number; passages_without_embeddings: number };
  partial_results: boolean;
};
export type ProcessingSettings = {
  recording_context: string;
  voice_thresholds: { strong: number; margin: number };
  models: Record<string, string>;
  voice_configured: boolean;
  recorder_timezone: string;
};
export type Authorization = {
  request_id: string;
  client_name: string;
  scope: string;
  redirect_uri: string;
};

/** A share link: whoever holds `url` can open the recording until `expires_at` (null: never). */
export type Share = { url: string; created_at: string; expires_at: string | null };
export type SharedLink = Share & { entry_id: string; title: string };
export type Expiry = '1d' | '7d' | '30d' | 'never';
/** A recording's link while it works, or null; the Share button and dialog read the same. */
export const shareQuery = (id: string) =>
  queryOptions({
    queryKey: ['share', id],
    queryFn: async ({ client, signal }) => {
      const known = cachedLinks(client, id);
      const since = leaving();
      const next = await api<Share | null>(`/v1/entries/${id}/share`, { signal });
      landed++;
      if (next) confirm(next.url, since);
      forgetMissing(known, new Set(next ? [next.url] : []), since);
      return next;
    },
  });
/** Every link that works (Settings → Shared links). */
export const sharesQuery = queryOptions({
  queryKey: ['shares'],
  queryFn: async ({ client, signal }) => {
    const known = cachedLinks(client);
    const since = leaving();
    const next = await api<{ shares: SharedLink[] }>('/v1/shares', { signal });
    landed++;
    for (const link of next.shares) confirm(link.url, since);
    forgetMissing(known, new Set(next.shares.map((link) => link.url)), since);
    return next;
  },
});
/** The links this page holds for a recording (`entry`), or for every recording: from the
 * recording's own read and from Settings' list. Taken before a read, so a link the server no
 * longer answers with is known to be gone (stopped or replaced: its token never works again) for
 * every reader, while a link made during the read, which the read may predate, is not in it. A
 * read that was cancelled throws before it gets that far. */
export function cachedLinks(client: QueryClient, entry?: string): Share[] {
  const links: Share[] = [];
  for (const [key, share] of client.getQueriesData<Share | null>({ queryKey: ['share'] }))
    if (share && (entry === undefined || key[1] === entry)) links.push(share);
  for (const link of client.getQueryData<{ shares: SharedLink[] }>(['shares'])?.shares ?? [])
    if (entry === undefined || link.entry_id === entry) links.push(link);
  return links;
}
/** A link past its expiry by this page's clock. Its absence from a read says nothing more than
 * the clock does, and the page clock already stops offering it, while taking it for gone would
 * hide an extension made at its deadline that a read could have just missed. */
const lapsed = (link: Share) => !!link.expires_at && Date.parse(link.expires_at) <= Date.now();
/** Takes for gone each held link a read that left at `since` no longer has, unless any copy the
 * page holds of it has merely lapsed (an older copy in the other view may not show the deadline
 * it has since read), or a read or change that left later was answered with it (`missing`). */
function forgetMissing(known: Share[], answered: Set<string>, since: number) {
  const lapsedUrls = new Set(known.filter(lapsed).map((link) => link.url));
  for (const link of known)
    if (!answered.has(link.url) && !lapsedUrls.has(link.url)) missing(link.url, since);
}
/** Share reads (of a recording's link or of Settings' list) and change answers that have landed
 * on this page. It only grows: a list dropped from memory and read afresh counts on from here,
 * unlike the cache's own per-query counts. */
let landed = 0;
/** How many share reads and answers had landed as a change went out, for `keepAnswer`: each
 * counts, even two in one millisecond. */
export const sentAt = () => landed;
/** Puts the answer to a change of the recording's link (`sent`: `sentAt` as it went out) in the
 * cache, unless a read of the link or of Settings' list, or another change's answer, landed after
 * the change was sent: that may be newer than this answer (another tab shared again, or a later
 * change answered first), so the page asks again instead of overwriting it.
 *
 * ponytail: tabs do not tell each other what they changed; each page reconciles with the server
 * when it reads, so another tab's change shows here at this page's next read (offline, not until
 * it can read). A BroadcastChannel between tabs is the upgrade if that ever matters. */
export async function keepAnswer(
  client: QueryClient,
  id: string,
  value: Share | null,
  sent: number,
  /** `leaving()` as the change went out. */
  since: number,
) {
  // The server answered with this link working: if a racing read took it for gone before the
  // change went out, it was not.
  if (value) confirm(value.url, since);
  // A read still under way would land after this and bring back what was just changed.
  await client.cancelQueries({ queryKey: ['share', id] });
  if (landed === sent) {
    landed++;
    client.setQueryData(['share', id], value);
  } else await client.invalidateQueries({ queryKey: ['share', id] });
  return client.invalidateQueries({ queryKey: ['shares'] });
}
/** The token at the end of a link's URL, which names the link when it is changed or stopped. */
export const linkToken = (share: Pick<Share, 'url'>) =>
  share.url.slice(share.url.lastIndexOf('/') + 1);
/** The answer for a link that is no longer there: stopped, expired or replaced meanwhile. */
export const gone = (error: unknown) => error instanceof ApiError && error.status === 404;
/** A recording as its share link shows it: no labels, sources or speaker evidence. */
export type SharedEntry = Pick<
  Entry,
  | 'title'
  | 'recorded_at'
  | 'duration_ms'
  | 'stage'
  | 'summary'
  | 'summary_manual'
  | 'audio_available'
> & {
  video: Omit<NonNullable<Entry['video']>, 'failure'> | null;
  speakers: Pick<Speaker, 'label' | 'name'>[];
};

/** A share link's own data (`/s/{token}/…`), read with the link alone, never a session. */
export async function shared<T>(token: string, path: string, signal?: AbortSignal): Promise<T> {
  const r = await fetch(`/s/${encodeURIComponent(token)}/${path}`, { credentials: 'omit', signal });
  if (!r.ok) throw new ApiError(r.status, `Request failed (${r.status})`);
  return await r.json();
}

/** Every transcript page, in order. The server caps page size; follow next_cursor until done. */
export async function transcript(id: string, signal?: AbortSignal): Promise<Segment[]> {
  const rows: Segment[] = [];
  let cursor: number | null = 0;
  while (cursor !== null) {
    const page: Page<'segments', Segment> = await api(
      `/v1/entries/${id}/transcript?limit=2000&cursor=${cursor}`,
      { signal },
    );
    rows.push(...page.segments);
    cursor = page.next_cursor !== null && page.next_cursor > cursor ? page.next_cursor : null;
  }
  return rows;
}

export const isProcessing = (e: Pick<Entry, 'stage'>) =>
  !['complete', 'failed', 'paused'].includes(e.stage);

export const peopleQuery = queryOptions({
  queryKey: ['people'],
  queryFn: () => api<{ people: Person[] }>('/v1/people'),
});

/** Labels still in use; the server keeps a label after its last recording drops it. */
export const labelsQuery = queryOptions({
  queryKey: ['labels'],
  queryFn: () => api<{ labels: Label[] }>('/v1/labels'),
  select: (data) => data.labels.filter((label) => label.count > 0),
});
