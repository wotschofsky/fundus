<script lang="ts">
  import { createQuery, keepPreviousData, useQueryClient } from '@tanstack/svelte-query';
  import {
    Download,
    FileAudio,
    Globe,
    Link,
    Pencil,
    RefreshCw,
    Sparkles,
    Trash2,
  } from '@lucide/svelte';
  import {
    ApiError,
    api,
    isProcessing,
    peopleQuery,
    send,
    shareQuery,
    transcript,
    type Entry,
    type Person,
    type Segment,
  } from '../api';
  import { dateTime, downloadTranscript, duration, speakerNames } from '../format';
  import { Playback } from '../playback.svelte';
  import { router } from '../router.svelte';
  import BackLink from '../components/back-link.svelte';
  import ConfirmDialog, { type ConfirmRequest } from '../components/confirm-dialog.svelte';
  import EditDetailsForm, { type EntryEdit } from '../components/edit-details-form.svelte';
  import ErrorNotice from '../components/error-notice.svelte';
  import LoadingState from '../components/loading-state.svelte';
  import Player from '../components/player.svelte';
  import ProcessingNotice from '../components/processing-notice.svelte';
  import ShareDialog from '../components/share-dialog.svelte';
  import SpeakerControl from '../components/speaker-control.svelte';
  import SpeakerCorrection, {
    type CorrectionReceipt,
  } from '../components/speaker-correction.svelte';
  import TranscriptTurn from '../components/transcript-turn.svelte';
  import { LinkClock } from '../share-link.svelte';

  let { id }: { id: string } = $props();
  const qc = useQueryClient();
  let confirming: ConfirmRequest | null = $state(null);
  let editing = $state(false);
  let sharing = $state(false);
  let actionError: unknown = $state(null);
  let busy = $state(false);
  let correcting: Segment | null = $state(null);
  let correctionNotice = $state<'saved' | 'undone' | null>(null);
  let correctionReceipt: CorrectionReceipt | null = $state(null);
  let notThisSpeaker = $state(false);

  const entry = createQuery(() => ({
    queryKey: ['entry', id],
    queryFn: () => api<Entry>(`/v1/entries/${id}`),
    // Also while the video is being prepared, which goes on after the transcript is done.
    refetchInterval: ({ state: { data } }) =>
      data && (isProcessing(data) || data.video?.state === 'preparing') ? 3000 : false,
  }));
  const stage = $derived(entry.data?.stage);
  // Segments only change when processing moves on, so the stage is part of the key.
  const segments = createQuery(() => ({
    queryKey: ['transcript', id, stage],
    queryFn: ({ signal }) => transcript(id, signal),
    enabled: !!stage,
    placeholderData: keepPreviousData,
  }));
  const undo = createQuery(() => ({
    queryKey: ['correction-undo', id, stage],
    queryFn: () => api<{ available: boolean }>(`/v1/entries/${id}/speaker-splits/undo`),
    enabled: !!entry.data,
    staleTime: 0,
  }));
  const people = createQuery(() => peopleQuery);
  const share = createQuery(() => shareQuery(id));
  const clock = new LinkClock(
    () => [share.data?.expires_at],
    () => share.refetch(),
  );
  // Shared only while the link works by this page's clock, so an expired one is never offered.
  const offered = $derived(clock.offered(share.data));
  const rows = $derived(segments.data ?? []);
  const firstAppearances = $derived.by(() => {
    const first = new Map<string, Segment>();
    for (const row of rows) {
      if (!first.has(row.speaker)) first.set(row.speaker, row);
    }
    return first;
  });
  const names = $derived(speakerNames(entry.data?.speakers));
  const startMs = $derived(Number(router.url.searchParams.get('t')) * 1000 || 0);
  const playback = new Playback(
    () => rows,
    () => startMs,
  );

  const refresh = () =>
    Promise.all(
      ['entry', 'entries', 'labels', 'search', 'correction-undo'].map((key) =>
        qc.invalidateQueries({
          queryKey: ['entry', 'correction-undo'].includes(key) ? [key, id] : [key],
        }),
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
  function jumpToAppearance(segment: Segment) {
    const target = document.getElementById(segment.id);
    if (!target) return;
    target.focus({ preventScroll: true });
    const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
    target.scrollIntoView({ block: 'center', behavior: reducedMotion ? 'instant' : 'smooth' });
  }
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
      [['entry', id], ['entries'], ['people'], ['correction-undo', id]].map((queryKey) =>
        qc.invalidateQueries({ queryKey }),
      ),
    );
  };
  const moveToTrash = () =>
    (confirming = {
      title: 'Move this recording to Trash?',
      text: `${entry.data && isProcessing(entry.data) ? 'Processing stops after its current step. ' : ''}It disappears from your library, search and chat apps${offered ? ', and its share link stops working' : ''}. You can restore it from Trash.`,
      action: 'Move to Trash',
      run: async () => {
        await api(`/v1/entries/${id}/trash`, send('POST'));
        await refresh();
        qc.removeQueries({ queryKey: ['share', id] });
        await qc.invalidateQueries({ queryKey: ['shares'] });
        router.navigate('/');
      },
    });
  function startCorrection(row: Segment, fromPassage: boolean) {
    notThisSpeaker = fromPassage;
    correcting = row;
  }
  async function refreshCorrectionData() {
    await Promise.all(
      [
        ['entry', id],
        ['transcript', id],
        ['people'],
        ['entries'],
        ['search'],
        ['correction-undo', id],
      ].map((queryKey) => qc.invalidateQueries({ queryKey }, { throwOnError: true })),
    );
  }
  async function correctionApplied(receipt: CorrectionReceipt) {
    correctionReceipt = receipt;
    correctionNotice = 'saved';
    await refreshCorrectionData();
  }
  const undoCorrection = () =>
    act(async () => {
      await api(`/v1/entries/${id}/speaker-splits/undo`, send('POST'));
      correctionReceipt = null;
      correctionNotice = 'undone';
      await refreshCorrectionData();
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
      <button aria-expanded={editing} onclick={() => (editing = !editing)}>
        <Pencil size={14} aria-hidden="true" />
        Edit details
      </button>
      {#if rows.length > 0}
        <button
          onclick={() => downloadTranscript(e, rows, names)}
          title="Save the transcript as a Markdown file"
        >
          <Download size={14} aria-hidden="true" />
          Download
        </button>
      {/if}
      <button
        aria-haspopup="dialog"
        title={offered
          ? 'Anyone with the link can open this recording'
          : 'Only you can open this recording'}
        onclick={() => (sharing = true)}
      >
        {#if offered}
          <Globe size={14} aria-hidden="true" />
          Shared
        {:else}
          <Link size={14} aria-hidden="true" />
          Share
        {/if}
      </button>
      {#if !processing && rows.length > 0}
        <button
          disabled={busy}
          onclick={post('reanalyze')}
          title="Recognize the speakers again and rewrite the title and summary"
        >
          <RefreshCw size={14} aria-hidden="true" />
          Reanalyze speakers
        </button>
        <button
          disabled={busy}
          onclick={post('summarize')}
          title="Write the title and summary again; the speakers stay as they are"
        >
          <Sparkles size={14} aria-hidden="true" />
          Regenerate summary
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
  <ShareDialog {id} title={e.title} bind:open={sharing} />
  <ErrorNotice error={actionError} />
  {#if correctionNotice || undo.data?.available}
    <div class="notice correction-notice" role="status">
      <span
        >{correctionNotice === 'undone'
          ? 'Speaker correction undone.'
          : correctionNotice === 'saved'
            ? 'Speaker correction saved.'
            : 'Last speaker correction'}
        {#if correctionReceipt?.summary_refresh && processing}
          Updating summary…{/if}
        {#if correctionNotice === 'saved' && correctionReceipt && !correctionReceipt.summary_refresh}
          Automatic summaries aren’t configured.{/if}
      </span>
      {#if undo.data?.available}<button class="text-button" disabled={busy} onclick={undoCorrection}
          >Undo</button
        >{/if}
    </div>
  {/if}
  <ProcessingNotice entry={e} {busy} onresume={post('retry')} />
  {#if e.warnings.length > 0}
    <details class="notice">
      <summary>{e.warnings.length} processing note{e.warnings.length !== 1 ? 's' : ''}</summary>
      {#each e.warnings as w (w)}<p>{w}</p>{/each}
    </details>
  {/if}
  <Player
    {playback}
    base={`/api/v1/entries/${id}`}
    audio={e.audio_available}
    video={e.video}
    {busy}
    onretry={post('video/retry')}
  />
  <div class="detail-grid">
    <div class="detail-main">
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
            oncorrect={processing ? undefined : (row) => startCorrection(row, true)}
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
              firstAppearance={firstAppearances.get(s.label)}
              onjump={jumpToAppearance}
              onsplit={!processing && firstAppearances.has(s.label)
                ? () => startCorrection(firstAppearances.get(s.label)!, false)
                : undefined}
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
<SpeakerCorrection
  bind:segment={correcting}
  entryId={id}
  {notThisSpeaker}
  {names}
  playable={entry.data?.audio_available ?? false}
  onlisten={playback.listen}
  onstoplisten={playback.stopListening}
  onapplied={correctionApplied}
/>
<ConfirmDialog bind:request={confirming} />
