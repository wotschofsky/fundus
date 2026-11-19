<!-- A recording as its share link shows it: the player, summary and transcript, read-only. -->
<script lang="ts">
  import { createQuery, keepPreviousData, useQueryClient } from '@tanstack/svelte-query';
  import { Download } from '@lucide/svelte';
  import { gone, isProcessing, shared, type Segment, type SharedEntry } from '../api';
  import { dateTime, downloadTranscript, duration, speakerNames, startAt } from '../format';
  import { Playback } from '../playback.svelte';
  import EmptyState from '../components/empty-state.svelte';
  import ErrorNotice from '../components/error-notice.svelte';
  import LoadingState from '../components/loading-state.svelte';
  import Player from '../components/player.svelte';
  import TranscriptTurn from '../components/transcript-turn.svelte';

  let { token }: { token: string } = $props();
  /** A read found the link gone. A link that stopped working never works again, so the page stays
   * closed and stops asking, whatever a later attempt (a failed one, say) would leave behind. */
  let dead = $state(false);
  const entry = createQuery(() => ({
    queryKey: ['shared', token],
    queryFn: ({ signal }) => shared<SharedEntry>(token, 'entry', signal),
    enabled: !dead,
    // Like the owner's page: while the recording, then its video, is being prepared; never
    // after the link stopped working, though the last answer is still at hand.
    refetchInterval: ({ state: { data, error } }) =>
      !gone(error) && data && (isProcessing(data) || data.video?.state === 'preparing')
        ? 3000
        : false,
  }));
  const stage = $derived(entry.data?.stage);
  // Processing moving on changes the segments, so the stage is part of the key. The owner's
  // speaker corrections change them too; they show when the page asks again, on coming back to it.
  const segments = createQuery(() => ({
    queryKey: ['shared-transcript', token, stage],
    // The whole transcript in one read, so no correction falls between parts of it.
    queryFn: async ({ signal }): Promise<Segment[]> =>
      (await shared<{ segments: Segment[] }>(token, 'transcript', signal)).segments,
    enabled: !!stage && !dead && !gone(entry.error),
    placeholderData: keepPreviousData,
  }));
  const qc = useQueryClient();
  $effect(() => {
    if (!dead && (gone(entry.error) || gone(segments.error))) {
      dead = true;
      // A retry already scheduled would still run: cancelled, so nothing more is asked.
      qc.cancelQueries({ queryKey: ['shared', token] });
      qc.cancelQueries({ queryKey: ['shared-transcript', token] });
    }
  });
  /** Either read finding the link gone closes the page for good, which takes the player away.
   * The player's own requests failing does not. */
  const closed = $derived(dead || gone(entry.error) || gone(segments.error));
  const rows = $derived(segments.data ?? []);
  // A name read with the transcript's turns wins over the recording's, as it matches them.
  const names = $derived(speakerNames(entry.data?.speakers, rows));
  // A link can point at a moment, like a search result's: `?t=` in seconds.
  const startMs = startAt(new URLSearchParams(location.search).get('t'));
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
      <!-- Says there is no transcript only once this stage's read has answered: a first read paused
           offline is not fetching, and the previous stage's rows stand in while it reads. -->
      {#if !segments.isSuccess || segments.isPlaceholderData || segments.isFetching}
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
