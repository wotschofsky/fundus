<script lang="ts">
  import { createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { Trash2, Upload } from '@lucide/svelte';
  import { labelsQuery, type Entry } from '../api';
  import { createEntries } from '../entries';
  import { day, dayKey, duration, outcome, time } from '../format';
  import { router } from '../router.svelte';
  import { uploads } from '../uploads.svelte';
  import EmptyState from '../components/empty-state.svelte';
  import ErrorNotice from '../components/error-notice.svelte';
  import LoadMoreButton from '../components/load-more-button.svelte';
  import LoadingState from '../components/loading-state.svelte';
  import StageLabel from '../components/stage-label.svelte';

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

  // Filters live in the URL, so Back from a recording returns to the same list.
  const label = $derived(router.url.searchParams.get('label') ?? '');
  const stage = $derived(router.url.searchParams.get('stage') ?? '');
  const entries = createEntries(
    () => `&labels=${encodeURIComponent(label)}&stage=${encodeURIComponent(stage)}`,
    true,
  );
  const labels = createQuery(() => labelsQuery);
  const all = $derived(entries.data ?? []);
  const groups = $derived(byDay(all));
  const filtered = $derived(!!(label || stage));

  const queryClient = useQueryClient();
  let picker: HTMLInputElement;
  function start(files: File[]) {
    uploads.add(files).then(() => queryClient.invalidateQueries({ queryKey: ['entries'] }));
  }

  // Files dropped anywhere on the page upload. A depth count, since every child fires its own
  // dragenter and dragleave; links and text being dragged are left alone.
  let depth = $state(0);
  const carriesFiles = (event: DragEvent) => !!event.dataTransfer?.types.includes('Files');
  function ondragenter(event: DragEvent) {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    depth++;
  }
  function ondragleave(event: DragEvent) {
    if (carriesFiles(event)) depth = Math.max(0, depth - 1);
  }
  function ondragover(event: DragEvent) {
    if (carriesFiles(event)) event.preventDefault();
  }
  function ondrop(event: DragEvent) {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    depth = 0;
    start([...event.dataTransfer!.files]);
  }
</script>

<svelte:window {ondragenter} {ondragleave} {ondragover} {ondrop} />

{#if depth}
  <div class="drop-overlay" aria-hidden="true"><p>Drop audio files to upload</p></div>
{/if}

<header class="page-header">
  <h1>Recordings</h1>
  <div class="header-links">
    <a class="text-link" href="/trash">
      <Trash2 size={15} aria-hidden="true" />
      Trash
    </a>
    <a class="text-link" href="/settings#connect">Connect apps</a>
    <button class="primary" onclick={() => picker.click()}>
      <Upload size={15} aria-hidden="true" />
      Upload
    </button>
    <input
      bind:this={picker}
      type="file"
      accept="audio/*"
      multiple
      hidden
      onchange={(event) => {
        start([...event.currentTarget.files!]);
        event.currentTarget.value = '';
      }}
    />
  </div>
</header>
<div class="toolbar">
  <span aria-live="polite">
    {entries.isPending
      ? ''
      : `${all.length}${entries.hasNextPage ? '+' : ''} recording${all.length === 1 && !entries.hasNextPage ? '' : 's'}`}
  </span>
  <div class="filters">
    <label>
      <span class="sr-only">Filter by label</span>
      <select value={label} onchange={(e) => router.setParam('label', e.currentTarget.value)}>
        <option value="">All labels</option>
        {#each labels.data ?? [] as l (l.name)}
          <option value={l.name}>{l.name} ({l.count})</option>
        {/each}
      </select>
    </label>
    <label>
      <span class="sr-only">Filter by status</span>
      <select value={stage} onchange={(e) => router.setParam('stage', e.currentTarget.value)}>
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
{#if entries.isPending}
  <LoadingState label="Loading recordings…" />
{:else if all.length === 0}
  {#if filtered}
    <EmptyState title="No matching recordings" text="Try another label or status." />
  {:else}
    <EmptyState
      title="No recordings yet"
      text="Upload or drop audio files here, or import them with the Mac or mobile app. They appear here while your server transcribes them."
    >
      <a class="button" href="/settings#connect">Connect an app</a>
    </EmptyState>
  {/if}
{:else}
  {#each groups as group (group.key)}
    <section class="day" aria-labelledby={`day-${group.key}`}>
      <h2 id={`day-${group.key}`}>{group.label}</h2>
      {#each group.entries as e (e.id)}
        <a class="recording" href={`/entries/${e.id}`}>
          <div class="recording-meta">
            <time datetime={e.recorded_at ?? undefined}>
              {e.recorded_at ? time(e.recorded_at) : 'Time unknown'}
            </time>
            <span>{duration(e.duration_ms)}</span>
            <StageLabel entry={e} />
          </div>
          <h3>{e.title}</h3>
          {#if e.summary || e.failure}
            <p class={e.summary ? '' : 'failure'}>{e.summary ? outcome(e.summary) : e.failure}</p>
          {/if}
          {#if e.labels.length > 0}
            <div class="chips">
              {#each e.labels as l (l)}<span>{l}</span>{/each}
            </div>
          {/if}
        </a>
      {/each}
    </section>
  {/each}
{/if}
<LoadMoreButton query={entries} />
