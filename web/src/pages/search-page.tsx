import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { ArrowRight, Search as SearchIcon } from 'lucide-react';
import { api, labelsQuery, type SearchResponse } from '../api';
import { date, timestamp } from '../format';
import { EmptyState } from '../components/empty-state';
import { ErrorNotice } from '../components/error-notice';
import { Highlight } from '../components/highlight';
import { LoadingState } from '../components/loading-state';

/** The instant a local calendar day (`YYYY-MM-DD`) starts, `days` later, for the server's filter. */
function localDay(day: string, days = 0) {
  const start = new Date(`${day}T00:00`);
  start.setDate(start.getDate() + days);
  return encodeURIComponent(start.toISOString());
}

export function SearchPage() {
  // The query and filters live in the URL, so Back and Forward restore a search.
  const [params, setParams] = useSearchParams();
  const q = params.get('q') ?? '';
  const from = params.get('from') ?? '';
  const to = params.get('to') ?? '';
  const label = params.get('label') ?? '';
  const [query, setQuery] = useState(q);
  useEffect(() => setQuery(q), [q]);
  const update = (key: string, value: string) =>
    setParams(
      (current) => {
        if (value) current.set(key, value);
        else current.delete(key);
        return current;
      },
      { replace: true },
    );
  const labels = useQuery(labelsQuery);
  const result = useQuery({
    queryKey: ['search', q, from, to, label],
    queryFn: () =>
      api<SearchResponse>(
        `/v1/search?q=${encodeURIComponent(q)}&limit=30${from ? `&from=${localDay(from)}` : ''}${to ? `&to=${localDay(to, 1)}` : ''}${label ? `&labels=${encodeURIComponent(label)}` : ''}`,
      ),
    enabled: !!q,
    placeholderData: keepPreviousData,
  });
  const data = result.data;
  // Without semantic search every passage is keyword-only; that is not a coverage gap.
  const missing = data?.mode === 'hybrid' ? data.coverage.passages_without_embeddings : 0;
  return (
    <>
      <header className="page-header">
        <h1>Search</h1>
      </header>
      <form
        className="search-form"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          if (!query.trim()) return;
          setParams((current) => {
            current.set('q', query.trim());
            return current;
          });
        }}
      >
        <SearchIcon size={18} aria-hidden />
        <input
          id="search-input"
          type="search"
          aria-label="Search transcripts"
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search what was said"
        />
        <button className="primary" disabled={!query.trim()}>
          Search
        </button>
      </form>
      <div className="search-filters">
        <label>
          From
          <input
            type="date"
            value={from}
            max={to || undefined}
            onChange={(e) => update('from', e.target.value)}
          />
        </label>
        <label>
          Through
          <input
            type="date"
            value={to}
            min={from || undefined}
            onChange={(e) => update('to', e.target.value)}
          />
        </label>
        <label>
          Label
          <select value={label} onChange={(e) => update('label', e.target.value)}>
            <option value="">Any label</option>
            {labels.data?.map((l) => (
              <option key={l.name} value={l.name}>
                {l.name} ({l.count})
              </option>
            ))}
          </select>
        </label>
      </div>
      <ErrorNotice error={result.error} />
      {!q ? (
        <EmptyState
          title="Find a moment"
          text="Search for a word, a phrase or a topic. Results link straight to that point in the recording."
        />
      ) : result.isPending ? (
        <LoadingState label="Searching…" />
      ) : data ? (
        <>
          <div className="toolbar">
            <span aria-live="polite">
              {data.results.length} passage{data.results.length !== 1 ? 's' : ''}
            </span>
            <span>{data.mode === 'hybrid' ? 'Keywords and meaning' : 'Keywords only'}</span>
          </div>
          {(data.coverage.pending_entries > 0 || missing > 0) && (
            <div className="notice">
              Results may be incomplete:{' '}
              {[
                data.coverage.pending_entries > 0 &&
                  `${data.coverage.pending_entries} recording${data.coverage.pending_entries !== 1 ? 's are' : ' is'} still processing`,
                missing > 0 &&
                  `${missing} passage${missing !== 1 ? 's' : ''} can only be found by keyword`,
              ]
                .filter(Boolean)
                .join('; ')}
              .
            </div>
          )}
          {data.results.length === 0 ? (
            <EmptyState
              title="No matching passages"
              text="Try other words, or browse your recordings by the date you remember."
            />
          ) : (
            <div className="rows">
              {data.results.map((r) => (
                <Link
                  key={r.id}
                  className="search-result"
                  to={`/entries/${r.entry_id}?t=${r.start_ms / 1000}`}
                >
                  <div className="recording-meta">
                    <span>{date(r.recorded_at)}</span>
                    <span>
                      {timestamp(r.start_ms)}–{timestamp(r.end_ms)}
                    </span>
                  </div>
                  <h3>
                    {r.title}
                    <ArrowRight size={14} aria-hidden />
                  </h3>
                  <p>
                    <Highlight text={r.excerpt} query={q} />
                  </p>
                </Link>
              ))}
            </div>
          )}
        </>
      ) : null}
    </>
  );
}
