<!-- A recording as its share link shows it: the player, summary and transcript, read-only. -->
<script lang="ts">
  import { createQuery, keepPreviousData } from '@tanstack/svelte-query';
  import { Download } from '@lucide/svelte';
  import { ApiError, allSegments, isProcessing, shared, type SharedEntry } from '../api';
  import { dateTime, downloadTranscript, duration, speakerNames } from '../format';
  import { Playback } from '../playback.svelte';
  import EmptyState from '../components/empty-state.svelte';
  import ErrorNotice from '../components/error-notice.svelte';
  import LoadingState from '../components/loading-state.svelte';
  import Player from '../components/player.svelte';
  import TranscriptTurn from '../components/transcript-turn.svelte';

  let { token }: { token: string } = $props();
  /** The link stopped working: it expired, sharing stopped, or the recording went to Trash. */
  const gone = (error: unknown) => error instanceof ApiError && error.status === 404;
  const entry = createQuery(() => ({
    queryKey: ['shared', token],
    queryFn: ({ signal }) => shared<SharedEntry>(token, 'entry', signal),
    // Like the owner's page: while the recording, then its video, is being prepared; never
    // after the link stopped working, though the last answer is still at hand.
    refetchInterval: ({ state: { data, error } }) =>
      !gone(error) && data && (isProcessing(data) || data.video?.state === 'preparing')
        ? 3000
        : false,
  }));
  const stage = $derived(entry.data?.stage);
  // Segments only change when processing moves on, so the stage is part of the key.
  const segments = createQuery(() => ({
    queryKey: ['shared-transcript', token, stage],
    queryFn: ({ signal }) =>
      allSegments((cursor) => shared(token, `transcript?limit=2000&cursor=${cursor}`, signal)),
    enabled: !!stage && !gone(entry.error),
    placeholderData: keepPreviousData,
  }));
  /** Whichever request finds out first closes the page, which takes the player away. */
  const closed = $derived(gone(entry.error) || gone(segments.error));
  const rows = $derived(segments.data ?? []);
  const names = $derived(speakerNames(entry.data?.speakers));
  // A link can point at a moment, like a search result's: `?t=` in seconds.
  const startMs = Number(new URLSearchParams(location.search).get('t')) * 1000 || 0;
  const playback = new Playback(
    () => rows,
    () => startMs,
  );
</script>

{#if entry.isPending}
  <LoadingState label="Loading recording…" />
{:else if closed}
  <EmptyState
    title="This link doesn’t work"
    text="It may have expired, or sharing was stopped. Ask whoever sent it for a new link."
  />
{:else if !entry.data}
  <ErrorNotice error={entry.error} />
{:else}
  {@const e = entry.data}
  <header class="detail-header">
    <div class="recording-meta">
      <time datetime={e.recorded_at ?? undefined}>{dateTime(e.recorded_at)}</time>
      <span>{duration(e.duration_ms)}</span>
    </div>
    <h1>{e.title}</h1>
    {#if rows.length > 0}
      <div class="detail-actions">
        <button
          onclick={() => downloadTranscript(e, rows, names)}
          title="Save the transcript as a Markdown file"
        >
          <Download size={14} aria-hidden="true" />
          Download
        </button>
      </div>
    {/if}
  </header>
  <Player
    {playback}
    base={`/s/${encodeURIComponent(token)}`}
    audio={e.audio_available}
    video={e.video}
  />
  {#if e.summary}
    <section class="summary">
      <div class="section-heading">
        <h2>Summary</h2>
        <span>{e.summary_manual ? 'Edited' : 'Generated'}</span>
      </div>
      <p>{e.summary}</p>
    </section>
  {/if}
  <section class="transcript" aria-labelledby="transcript-heading">
    <div class="section-heading">
      <h2 id="transcript-heading">Transcript</h2>
      {#if rows.length > 0 && e.audio_available}<span>Select a time to listen</span>{/if}
    </div>
    <ErrorNotice error={segments.error} />
    {#if rows.length === 0 && !segments.error}
      {#if segments.isFetching}
        <LoadingState label="Loading transcript…" />
      {:else}
        <p class="muted">
          {isProcessing(e)
            ? 'The transcript appears here when transcription finishes.'
            : 'No speech was transcribed for this recording.'}
        </p>
      {/if}
    {/if}
    {#each rows as r, i (r.id)}
      <TranscriptTurn
        row={r}
        name={names.get(r.speaker) ?? r.speaker_name ?? 'Speaker'}
        showName={i === 0 || rows[i - 1].speaker !== r.speaker}
        active={i === playback.active}
        playable={e.audio_available}
        onseek={playback.seek}
      />
    {/each}
  </section>
{/if}
