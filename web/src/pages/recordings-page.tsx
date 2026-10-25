import { useMemo } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { Trash2 } from 'lucide-react';
import { labelsQuery, type Entry } from '../api';
import { day, dayKey, duration, time } from '../format';
import { useEntries } from '../hooks/use-entries';
import { EmptyState } from '../components/empty-state';
import { ErrorNotice } from '../components/error-notice';
import { LoadMoreButton } from '../components/load-more-button';
import { LoadingState } from '../components/loading-state';
import { StageLabel } from '../components/stage-label';

/** Consecutive entries (already newest first) grouped by their calendar day. */
function byDay(entries: Entry[]) {
  const groups: { key: string; label: string; entries: Entry[] }[] = [];
  for (const e of entries) {
    const when = e.recorded_at ?? e.created_at;
    const key = dayKey(when);
    if (groups.at(-1)?.key !== key) groups.push({ key, label: day(when), entries: [] });
    groups.at(-1)!.entries.push(e);
  }
  return groups;
}

export function RecordingsPage() {
  // Filters live in the URL, so Back from a recording returns to the same list.
  const [params, setParams] = useSearchParams();
  const label = params.get('label') ?? '';
  const stage = params.get('stage') ?? '';
  const filter = (key: string, value: string) =>
    setParams(
      (current) => {
        if (value) current.set(key, value);
        else current.delete(key);
        return current;
      },
      { replace: true },
    );
  const entries = useEntries(
    `&labels=${encodeURIComponent(label)}&stage=${encodeURIComponent(stage)}`,
    true,
  );
  const labels = useQuery(labelsQuery);
  const all = useMemo(() => entries.data ?? [], [entries.data]);
  const groups = useMemo(() => byDay(all), [all]);
  const filtered = !!(label || stage);
  return (
    <>
      <header className="page-header">
        <h1>Recordings</h1>
        <div className="header-links">
          <Link className="text-link" to="/trash">
            <Trash2 size={15} aria-hidden />
            Trash
          </Link>
          <Link className="text-link" to="/settings#connect">
            Connect apps
          </Link>
        </div>
      </header>
      <div className="toolbar">
        <span aria-live="polite">
          {entries.isPending
            ? ''
            : `${all.length}${entries.hasNextPage ? '+' : ''} recording${all.length === 1 && !entries.hasNextPage ? '' : 's'}`}
        </span>
        <div className="filters">
          <label>
            <span className="sr-only">Filter by label</span>
            <select value={label} onChange={(e) => filter('label', e.target.value)}>
              <option value="">All labels</option>
              {labels.data?.map((l) => (
                <option key={l.name} value={l.name}>
                  {l.name} ({l.count})
                </option>
              ))}
            </select>
          </label>
          <label>
            <span className="sr-only">Filter by status</span>
            <select value={stage} onChange={(e) => filter('stage', e.target.value)}>
              <option value="">Any status</option>
              <option value="complete">Ready</option>
              <option value="queued">Queued</option>
              <option value="paused">Paused</option>
              <option value="failed">Needs attention</option>
            </select>
          </label>
        </div>
      </div>
      <ErrorNotice error={entries.error} />
      {entries.isPending ? (
        <LoadingState label="Loading recordings…" />
      ) : all.length === 0 ? (
        filtered ? (
          <EmptyState title="No matching recordings" text="Try another label or status." />
        ) : (
          <EmptyState
            title="No recordings yet"
            text="Import recordings with the Mac or mobile app. They appear here while your server transcribes them."
          >
            <Link className="button" to="/settings#connect">
              Connect an app
            </Link>
          </EmptyState>
        )
      ) : (
        groups.map((group) => (
          <section className="day" key={group.key} aria-labelledby={`day-${group.key}`}>
            <h2 id={`day-${group.key}`}>{group.label}</h2>
            {group.entries.map((e) => (
              <Link className="recording" key={e.id} to={`/entries/${e.id}`}>
                <div className="recording-meta">
                  <time dateTime={e.recorded_at ?? undefined}>
                    {e.recorded_at ? time(e.recorded_at) : 'Time unknown'}
                  </time>
                  <span>{duration(e.duration_ms)}</span>
                  <StageLabel entry={e} />
                </div>
                <h3>{e.title}</h3>
                {(e.summary || e.failure) && (
                  <p className={e.summary ? '' : 'failure'}>{e.summary ?? e.failure}</p>
                )}
                {e.labels.length > 0 && (
                  <div className="chips">
                    {e.labels.map((l) => (
                      <span key={l}>{l}</span>
                    ))}
                  </div>
                )}
              </Link>
            ))}
          </section>
        ))
      )}
      <LoadMoreButton query={entries} />
    </>
  );
}
