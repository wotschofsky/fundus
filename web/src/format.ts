import type { Entry, Speaker, Stage } from './api';

const STAGES: Record<Stage, string> = {
  queued: 'Queued',
  preparing: 'Preparing audio',
  transcribing: 'Transcribing',
  recognizing: 'Recognizing speakers',
  enriching: 'Writing summary',
  indexing: 'Indexing for search',
  paused: 'Paused',
  failed: 'Needs attention',
  complete: 'Ready',
};
export const stageName = (e: Pick<Entry, 'stage' | 'pause_requested'>) =>
  e.pause_requested ? 'Pausing after this step' : (STAGES[e.stage] ?? e.stage);

/** Display names follow the speaker list order: "Speaker 1", "Speaker 2", unless identified. */
export function speakerNames(speakers: Speaker[] = []): Map<string, string> {
  return new Map(speakers.map((s, i) => [s.label, s.name ?? `Speaker ${i + 1}`]));
}

export const duration = (ms: number) => {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, '0')} min`;
};
export const timestamp = (ms: number) => {
  const s = Math.floor(ms / 1000);
  const mm = String(Math.floor((s % 3600) / 60)).padStart(2, '0');
  const ss = String(s % 60).padStart(2, '0');
  return s >= 3600 ? `${Math.floor(s / 3600)}:${mm}:${ss}` : `${mm}:${ss}`;
};

// Times are shown in the viewer's own zone. An entry's `timezone` is the recorder clock it was
// imported with (e.g. the DJI factory UTC+8), not where the person is.
const format = (value: string, options: Intl.DateTimeFormatOptions) =>
  new Intl.DateTimeFormat(undefined, options).format(new Date(value));
/** "24 Oct 2026" */
export const date = (value: string | null) =>
  value ? format(value, { dateStyle: 'medium' }) : 'Date unknown';
/** "14:05" */
export const time = (value: string) => format(value, { hour: '2-digit', minute: '2-digit' });
/** "24 Oct 2026 · 14:05" */
export const dateTime = (value: string | null) =>
  value ? `${date(value)} · ${time(value)}` : 'Date unknown';
/** "Thursday, 23 October" (with the year when it is not this year). */
export const day = (value: string) =>
  format(value, {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    ...(new Date(value).getFullYear() !== new Date().getFullYear() ? { year: 'numeric' } : {}),
  });
/** Sortable calendar day, for grouping. */
export const dayKey = (value: string) => new Intl.DateTimeFormat('en-CA').format(new Date(value));

const pad = (n: number) => String(n).padStart(2, '0');
/** ISO instant → value for <input type="datetime-local"> in the browser's own time zone. */
export const localInput = (iso: string | null) => {
  if (!iso) return '';
  const d = new Date(iso);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
};
