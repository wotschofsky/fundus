<script lang="ts">
  import { createQuery, keepPreviousData } from '@tanstack/svelte-query';
  import { ArrowRight, Search as SearchIcon } from '@lucide/svelte';
  import { api, labelsQuery, type SearchResponse } from '../api';
  import { date, timestamp } from '../format';
  import { router } from '../router.svelte';
  import EmptyState from '../components/empty-state.svelte';
  import ErrorNotice from '../components/error-notice.svelte';
  import Highlight from '../components/highlight.svelte';
  import LoadingState from '../components/loading-state.svelte';

  /** The instant a local calendar day (`YYYY-MM-DD`) starts, `days` later, for the server's filter. */
  function localDay(day: string, days = 0) {
    const start = new Date(`${day}T00:00`);
    start.setDate(start.getDate() + days);
    return encodeURIComponent(start.toISOString());
  }

  // The query and filters live in the URL, so Back and Forward restore a search.
  const params = $derived(router.url.searchParams);
  const q = $derived(params.get('q') ?? '');
  const from = $derived(params.get('from') ?? '');
  const to = $derived(params.get('to') ?? '');
  const label = $derived(params.get('label') ?? '');
  /** What is typed; follows the URL when Back or Forward changes the search. */
  let query = $derived(q);
  const labels = createQuery(() => labelsQuery);
  const result = createQuery(() => ({
    queryKey: ['search', q, from, to, label],
    queryFn: () =>
      api<SearchResponse>(
        `/v1/search?q=${encodeURIComponent(q)}&limit=30${from ? `&from=${localDay(from)}` : ''}${to ? `&to=${localDay(to, 1)}` : ''}${label ? `&labels=${encodeURIComponent(label)}` : ''}`,
      ),
    enabled: !!q,
    placeholderData: keepPreviousData,
  }));
  const data = $derived(result.data);
  // Without semantic search every passage is keyword-only; that is not a coverage gap.
  const missing = $derived(data?.mode === 'hybrid' ? data.coverage.passages_without_embeddings : 0);
  const incomplete = $derived(
    data
      ? [
          data.coverage.pending_entries > 0 &&
            `${data.coverage.pending_entries} recording${data.coverage.pending_entries !== 1 ? 's are' : ' is'} still processing`,
          missing > 0 &&
            `${missing} passage${missing !== 1 ? 's' : ''} can only be found by keyword`,
        ]
          .filter(Boolean)
          .join('; ')
      : '',
  );
</script>

<header class="page-header">
  <h1>Search</h1>
</header>
<form
  class="search-form"
  role="search"
  onsubmit={(e) => {
    e.preventDefault();
    if (query.trim()) router.setParam('q', query.trim(), { replace: false });
  }}
>
  <SearchIcon size={18} aria-hidden="true" />
  <input
    id="search-input"
    type="search"
    aria-label="Search transcripts"
    {@attach (input) => input.focus()}
    value={query}
    oninput={(e) => (query = e.currentTarget.value)}
    placeholder="Search what was said"
  />
  <button class="primary" disabled={!query.trim()}>Search</button>
</form>
<div class="search-filters">
  <label>
    From
    <input
      type="date"
      value={from}
      max={to || undefined}
      onchange={(e) => router.setParam('from', e.currentTarget.value)}
    />
  </label>
  <label>
    Through
    <input
      type="date"
      value={to}
      min={from || undefined}
      onchange={(e) => router.setParam('to', e.currentTarget.value)}
    />
  </label>
  <label>
    Label
    <select value={label} onchange={(e) => router.setParam('label', e.currentTarget.value)}>
      <option value="">Any label</option>
      {#each labels.data ?? [] as l (l.name)}
        <option value={l.name}>{l.name} ({l.count})</option>
      {/each}
    </select>
  </label>
</div>
<ErrorNotice error={result.error} />
{#if !q}
  <EmptyState
    title="Find a moment"
    text="Search for a word, a phrase or a topic. Results link straight to that point in the recording."
  />
{:else if result.isPending}
  <LoadingState label="Searching…" />
{:else if data}
  <div class="toolbar">
    <span aria-live="polite">
      {data.results.length} passage{data.results.length !== 1 ? 's' : ''}
    </span>
    <span>{data.mode === 'hybrid' ? 'Keywords and meaning' : 'Keywords only'}</span>
  </div>
  {#if incomplete}
    <div class="notice">Results may be incomplete: {incomplete}.</div>
  {/if}
  {#if data.results.length === 0}
    <EmptyState
      title="No matching passages"
      text="Try other words, or browse your recordings by the date you remember."
    />
  {:else}
    <div class="rows">
      {#each data.results as r (r.id)}
        <a class="search-result" href={`/entries/${r.entry_id}?t=${r.start_ms / 1000}`}>
          <div class="recording-meta">
            <span>{date(r.recorded_at)}</span>
            <span>{timestamp(r.start_ms)}–{timestamp(r.end_ms)}</span>
          </div>
          <h3>{r.title}<ArrowRight size={14} aria-hidden="true" /></h3>
          <p><Highlight text={r.excerpt} query={q} /></p>
        </a>
      {/each}
    </div>
  {/if}
{/if}
