<script lang="ts">
  import { createQuery, keepPreviousData, useQueryClient } from '@tanstack/svelte-query';
  import { FileAudio, RefreshCw, Trash2 } from '@lucide/svelte';
  import {
    ApiError,
    api,
    isProcessing,
    peopleQuery,
    send,
    transcript,
    type Entry,
    type Person,
  } from '../api';
  import { dateTime, duration, speakerNames } from '../format';
  import { Playback } from '../playback.svelte';
  import { router } from '../router.svelte';
  import BackLink from '../components/back-link.svelte';
  import ConfirmDialog, { type ConfirmRequest } from '../components/confirm-dialog.svelte';
  import EditDetailsForm, { type EntryEdit } from '../components/edit-details-form.svelte';
  import ErrorNotice from '../components/error-notice.svelte';
  import LoadingState from '../components/loading-state.svelte';
  import ProcessingNotice from '../components/processing-notice.svelte';
  import SpeakerControl from '../components/speaker-control.svelte';
  import TranscriptTurn from '../components/transcript-turn.svelte';

  let { id }: { id: string } = $props();
  const qc = useQueryClient();
  let confirming: ConfirmRequest | null = $state(null);
  let editing = $state(false);
  let actionError: unknown = $state(null);
  let busy = $state(false);

  const entry = createQuery(() => ({
    queryKey: ['entry', id],
    queryFn: () => api<Entry>(`/v1/entries/${id}`),
    refetchInterval: (q) => (q.state.data && isProcessing(q.state.data) ? 3000 : false),
  }));
  const stage = $derived(entry.data?.stage);
  // Segments only change when processing moves on, so the stage is part of the key.
  const segments = createQuery(() => ({
    queryKey: ['transcript', id, stage],
    queryFn: ({ signal }) => transcript(id, signal),
    enabled: !!stage,
    placeholderData: keepPreviousData,
  }));
  const people = createQuery(() => peopleQuery);
  const rows = $derived(segments.data ?? []);
  const names = $derived(speakerNames(entry.data?.speakers));
  const startMs = $derived(Number(router.url.searchParams.get('t')) * 1000 || 0);
  const playback = new Playback(
    () => rows,
    () => startMs,
  );

  const refresh = () =>
    Promise.all(
      ['entry', 'entries', 'labels', 'search'].map((key) =>
        qc.invalidateQueries({ queryKey: key === 'entry' ? [key, id] : [key] }),
      ),
    );
  async function act(work: () => Promise<unknown>) {
    actionError = null;
    busy = true;
    try {
      await work();
    } catch (e) {
      actionError = e;
    } finally {
      busy = false;
    }
  }
  const post = (action: string) => () =>
    act(() => api(`/v1/entries/${id}/${action}`, send('POST')).then(refresh));
  const save = (edit: EntryEdit) =>
    act(async () => {
      await api(`/v1/entries/${id}`, send('PATCH', edit));
      await refresh();
      editing = false;
    });
  // Fails to the control, which shows the error beside it and keeps a typed name.
  // Names come from the entry's speakers, so the transcript itself is not refetched.
  const assign = (label: string) => async (personId: string | null, newName?: string) => {
    let person = personId;
    if (newName) person = (await api<Person>('/v1/people', send('POST', { name: newName }))).id;
    await api(
      `/v1/entries/${id}/speakers/${encodeURIComponent(label)}`,
      send('PATCH', { person_id: person }),
    );
    // Summaries show speaker names too, also in the recordings list.
    await Promise.all(
      [['entry', id], ['entries'], ['people']].map((queryKey) =>
        qc.invalidateQueries({ queryKey }),
      ),
    );
  };
  const moveToTrash = () =>
    (confirming = {
      title: 'Move this recording to Trash?',
      text: `${entry.data && isProcessing(entry.data) ? 'Processing stops after its current step. ' : ''}It disappears from your library, search and chat apps. You can restore it from Trash.`,
      action: 'Move to Trash',
      run: async () => {
        await api(`/v1/entries/${id}/trash`, send('POST'));
        await refresh();
        router.navigate('/');
      },
    });
</script>

{#if entry.isPending}
  <LoadingState label="Loading recording…" />
{:else if !entry.data}
  <BackLink />
  {#if entry.error instanceof ApiError && entry.error.status === 404}
    <div class="notice">This recording is not in your library. It may be in Trash or deleted.</div>
  {:else}
    <ErrorNotice error={entry.error} />
  {/if}
{:else}
  {@const e = entry.data}
  {@const processing = isProcessing(e)}
  <BackLink />
  <header class="detail-header">
    <div class="recording-meta">
      <time datetime={e.recorded_at ?? undefined}>{dateTime(e.recorded_at)}</time>
      <span>{duration(e.duration_ms)}</span>
    </div>
    <h1>{e.title}</h1>
    {#if e.labels.length > 0}
      <div class="chips">
        {#each e.labels as l (l)}<span>{l}</span>{/each}
      </div>
    {/if}
    <div class="detail-actions">
      <button aria-expanded={editing} onclick={() => (editing = !editing)}>Edit details</button>
      {#if !processing && rows.length > 0}
        <button
          disabled={busy}
          onclick={post('reanalyze')}
          title="Recognize the speakers again and rewrite the title and summary"
        >
          <RefreshCw size={14} aria-hidden="true" />
          Reanalyze speakers
        </button>
      {/if}
      <button class="danger" onclick={moveToTrash}>
        <Trash2 size={14} aria-hidden="true" />
        Move to Trash
      </button>
    </div>
  </header>
  {#if editing}
    <EditDetailsForm entry={e} onsave={save} oncancel={() => (editing = false)} />
  {/if}
  <ErrorNotice error={actionError} />
  <ProcessingNotice entry={e} {busy} onresume={post('retry')} />
  {#if e.warnings.length > 0}
    <details class="notice">
      <summary>{e.warnings.length} processing note{e.warnings.length !== 1 ? 's' : ''}</summary>
      {#each e.warnings as w (w)}<p>{w}</p>{/each}
    </details>
  {/if}
  {#if e.audio_available}
    <div class="player">
      <audio
        bind:this={playback.audio}
        ontimeupdate={playback.timeUpdate}
        onplay={() => (playback.playing = true)}
        onpause={() => (playback.playing = false)}
        onloadedmetadata={playback.loaded}
        controls
        preload="metadata"
        src={`/api/v1/entries/${id}/audio`}
      ></audio>
      <label>
        <input type="checkbox" bind:checked={playback.follow} />
        Follow transcript
      </label>
    </div>
  {/if}
  <div class="detail-grid">
    <div class="detail-main">
      {#if e.summary}
        <section class="summary">
          <div class="section-heading">
            <h2>Summary</h2>
            <span>Generated</span>
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
              {processing
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
    </div>
    <aside class="detail-side">
      {#if e.speakers?.length}
        <section>
          <h2>Speakers</h2>
          {#each e.speakers as s (s.label)}
            <SpeakerControl
              speaker={s}
              name={names.get(s.label)!}
              people={people.data?.people ?? []}
              onassign={assign(s.label)}
            />
          {/each}
        </section>
      {/if}
      {#if e.sources?.length}
        <section>
          <h2>Source files</h2>
          {#each e.sources as s, i (i)}
            <div class="source">
              <FileAudio size={14} aria-hidden="true" />
              <span>{s.filename}</span>
            </div>
          {/each}
        </section>
      {/if}
    </aside>
  </div>
{/if}
<ConfirmDialog bind:request={confirming} />
