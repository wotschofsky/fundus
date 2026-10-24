export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}
export async function api<T = any>(path: string, init: RequestInit = {}): Promise<T> {
  const r = await fetch(`/api${path}`, {
    ...init,
    headers: { ...(init.body ? { 'Content-Type': 'application/json' } : {}), ...init.headers },
  });
  if (!r.ok) {
    const v = await r.json().catch(() => ({ error: `Request failed (${r.status})` }));
    throw new ApiError(r.status, v.error_description ?? v.error);
  }
  return r.status === 204 ? (undefined as T) : await r.json();
}
export type Entry = {
  id: string;
  title: string;
  recorded_at: string | null;
  duration_ms: number;
  timezone: string;
  stage: string;
  failure: string | null;
  warnings: string[];
  summary: string | null;
  created_at: string;
  labels: string[];
  audio_available: boolean;
  speakers?: Speaker[];
  sources?: { offset_ms: number; recorded_at: string | null; filename: string }[];
};
export type Speaker = {
  label: string;
  person_id: string | null;
  name: string | null;
  state: string;
  evidence: any[];
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
export const duration = (ms: number) => {
  const s = Math.floor(ms / 1000);
  return s >= 3600
    ? `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`
    : `${Math.floor(s / 60)}m ${s % 60}s`;
};
export const timestamp = (ms: number) => {
  const s = Math.floor(ms / 1000);
  return `${s >= 3600 ? `${Math.floor(s / 3600)}:` : ''}${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
};
export const date = (value: string | null) =>
  value
    ? new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' }).format(new Date(value))
    : 'Recording date unknown';
