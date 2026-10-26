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
  created_at: string;
  trashed_at: string | null;
  labels: string[];
  audio_available: boolean;
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
export type Token = { id: string; name: string; scope: 'upload' | 'read'; created_at: string };
export type Connection = { id: string; name: string; scope: string; created_at: number };
export type SearchResult = {
  id: string;
  entry_id: string;
  title: string;
  recorded_at: string | null;
  start_ms: number;
  end_ms: number;
  excerpt: string;
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
