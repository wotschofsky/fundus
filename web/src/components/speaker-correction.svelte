<script module lang="ts">
  export type CorrectionReceipt = { changed: number; undo_id: string; summary_refresh: boolean };
</script>

<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { Play } from '@lucide/svelte';
  import { api, send, type Segment } from '../api';
  import { timestamp } from '../format';
  import ErrorNotice from './error-notice.svelte';
  import LoadingState from './loading-state.svelte';

  type Passage = Pick<Segment, 'id' | 'start_ms' | 'end_ms' | 'text'>;
  type Voice = { id: number; example: Passage; passages: Passage[] };
  type Split = {
    id: string;
    source_label: string;
    source_name: string;
    groups: Voice[];
    unchanged: number;
    suggested_keep: number | null;
  };
  let {
    segment = $bindable(),
    notThisSpeaker = false,
    entryId,
    names,
    playable,
    onlisten,
    onstoplisten,
    onapplied,
  }: {
    segment: Segment | null;
    notThisSpeaker?: boolean;
    entryId: string;
    names: Map<string, string>;
    playable: boolean;
    onlisten: (passage: Pick<Segment, 'start_ms' | 'end_ms'>) => void;
    onstoplisten: () => void;
    onapplied: (receipt: CorrectionReceipt) => Promise<unknown>;
  } = $props();
  let dialog: HTMLDialogElement;
  let split: Split | null = $state(null);
  let keepVoice: number | null = $state(null);
  let analyzing = $state(false);
  let saving = $state(false);
  let receipt: CorrectionReceipt | null = $state(null);
  let error: unknown = $state(null);
  let controller: AbortController | undefined;
  const sourceName = $derived(
    segment ? (names.get(segment.speaker) ?? 'this speaker') : 'this speaker',
  );
  const inferred = $derived.by(() => !!split && notThisSpeaker && split.suggested_keep != null);
  const moving = $derived.by(() => {
    if (keepVoice === null || !split) return null;
    return split.groups.find((g) => g.id !== keepVoice);
  });

  $effect(() => {
    if (segment && !dialog.open) {
      split = null;
      keepVoice = null;
      receipt = null;
      error = null;
      dialog.showModal();
      void analyze(segment.speaker, notThisSpeaker ? segment.id : undefined);
    }
    if (!segment) {
      controller?.abort();
      controller = undefined;
      onstoplisten();
      if (dialog.open) dialog.close();
    }
  });
  onDestroy(() => {
    controller?.abort();
    onstoplisten();
  });
  function close() {
    if (!saving) segment = null;
  }
  async function analyze(speaker: string, notId?: string) {
    controller?.abort();
    const request = new AbortController();
    controller = request;
    analyzing = true;
    error = null;
    try {
      const result = await api<Split>(`/v1/entries/${entryId}/speaker-splits`, {
        ...send('POST', { speaker, ...(notId ? { not_segment_id: notId } : {}) }),
        signal: request.signal,
      });
      if (!request.signal.aborted) {
        split = result;
        keepVoice = result.suggested_keep ?? null;
        await tick();
        if (!request.signal.aborted) {
          dialog
            .querySelector<HTMLElement>(
              result.suggested_keep == null ? 'input[type="radio"]' : 'button.primary',
            )
            ?.focus();
        }
      }
    } catch (e) {
      if (!request.signal.aborted) error = e;
    } finally {
      if (controller === request) analyzing = false;
    }
  }
  async function apply(event?: SubmitEvent) {
    event?.preventDefault();
    if (!split || keepVoice === null) return;
    saving = true;
    error = null;
    onstoplisten();
    try {
      if (!receipt) {
        receipt = await api<CorrectionReceipt>(
          `/v1/entries/${entryId}/speaker-splits/${split.id}/apply`,
          send('POST', { keep_group: keepVoice }),
        );
      }
      await onapplied(receipt);
      segment = null;
    } catch (e) {
      error = e;
    } finally {
      saving = false;
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="confirm speaker-correction-dialog"
  aria-labelledby="speaker-split-title"
  oncancel={(event) => {
    event.preventDefault();
    close();
  }}
>
  {#if segment}
    {#if split}
      <form onsubmit={apply}>
        {#if inferred && moving}
          <h2 id="speaker-split-title">Not {split.source_name}</h2>
          <p>
            {moving.passages.length} matching passage{moving.passages.length === 1 ? '' : 's'} will move
            to a separate speaker. The other passages will stay assigned to {split.source_name}.
          </p>
        {:else}
          <h2 id="speaker-split-title">Which voice is {split.source_name}?</h2>
          <p class="muted">
            {notThisSpeaker
              ? 'That passage was unclear. Listen to the samples and choose the voice that keeps this name.'
              : 'Listen to the samples and choose the voice that keeps this name.'}
          </p>
          <div class="split-voices" role="radiogroup" aria-labelledby="speaker-split-title">
            {#each split.groups as voice (voice.id)}
              <section class="split-voice">
                <div class="split-voice-heading">
                  <label>
                    <input
                      type="radio"
                      name="retained-voice"
                      value={voice.id}
                      bind:group={keepVoice}
                      disabled={saving || !!receipt}
                      required
                    />
                    <span
                      >Voice {voice.id + 1}
                      <small class="muted"
                        >{voice.passages.length} passage{voice.passages.length === 1
                          ? ''
                          : 's'}</small
                      ></span
                    >
                  </label>
                  {#if playable}<button
                      type="button"
                      class="text-button"
                      onclick={() => onlisten(voice.example)}
                      ><Play size={12} aria-hidden="true" /> Listen · {timestamp(
                        voice.example.start_ms,
                      )}</button
                    >{/if}
                </div>
                <p class="split-example">{voice.example.text}</p>
              </section>
            {/each}
          </div>
        {/if}
        {#if moving}
          <!-- svelte-ignore a11y_no_noninteractive_tabindex (Keyboard users need to scroll the passage list, including when audio is unavailable.) -->
          <div
            class="correction-passages"
            role="region"
            aria-label="Passages to separate"
            tabindex="0"
          >
            {#each moving.passages as passage (passage.id)}
              <div class="correction-passage">
                <div class="correction-context">
                  <span>{timestamp(passage.start_ms)}</span>
                  {#if playable}
                    <button
                      type="button"
                      class="text-button"
                      disabled={saving}
                      aria-label={`Listen to passage at ${timestamp(passage.start_ms)}`}
                      onclick={() => onlisten(passage)}
                      ><Play size={12} aria-hidden="true" /> Listen</button
                    >
                  {/if}
                </div>
                <p>{passage.text}</p>
              </div>
            {/each}
          </div>
        {/if}
        {#if split.unchanged}<p class="correction-hint muted">
            {split.unchanged} short or uncertain passage{split.unchanged === 1 ? '' : 's'} will stay unchanged.
          </p>{/if}
        <p class="correction-hint muted">
          You can name the new speaker afterward, or undo this correction.
        </p>
        <ErrorNotice {error} />
        <div class="dialog-actions">
          <button type="button" disabled={saving} onclick={close}>Cancel</button>
          <button class="primary" disabled={saving || keepVoice === null}>
            {saving
              ? 'Saving…'
              : receipt
                ? 'Refresh transcript'
                : inferred && moving
                  ? `Separate ${moving.passages.length} passage${moving.passages.length === 1 ? '' : 's'}`
                  : 'Apply split'}
          </button>
        </div>
      </form>
    {:else}
      <h2 id="speaker-split-title">
        {notThisSpeaker ? `Not ${sourceName}` : `Separate voices in ${sourceName}`}
      </h2>
      {#if analyzing}
        <LoadingState label="Finding matching passages…" />
        <p class="muted" role="status">Checking this speaker’s voice throughout the recording.</p>
      {:else}<ErrorNotice {error} />{/if}
      <div class="dialog-actions">
        <button onclick={close}>Cancel</button>
        {#if !analyzing}<button
            class="primary"
            onclick={() =>
              segment && analyze(segment.speaker, notThisSpeaker ? segment.id : undefined)}
            >Try again</button
          >{/if}
      </div>
    {/if}
  {/if}
</dialog>
