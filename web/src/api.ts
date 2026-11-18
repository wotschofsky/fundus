import { queryOptions } from '@tanstack/svelte-query';

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
    queryFn: () => api<Share | null>(`/v1/entries/${id}/share`),
  });
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
export async function allSegments(
  page: (cursor: number) => Promise<Page<'segments', Segment>>,
): Promise<Segment[]> {
  const rows: Segment[] = [];
  let cursor: number | null = 0;
  while (cursor !== null) {
    const next: Page<'segments', Segment> = await page(cursor);
    rows.push(...next.segments);
    cursor = next.next_cursor !== null && next.next_cursor > cursor ? next.next_cursor : null;
  }
  return rows;
}
export const transcript = (id: string, signal?: AbortSignal) =>
  allSegments((cursor) =>
    api(`/v1/entries/${id}/transcript?limit=2000&cursor=${cursor}`, { signal }),
  );

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
