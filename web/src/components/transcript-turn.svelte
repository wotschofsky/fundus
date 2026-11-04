<!-- One transcript segment. -->
<script lang="ts">
  import SpeakerSplitMenu from './speaker-split-menu.svelte';
  import type { Segment } from '../api';
  import { timestamp } from '../format';

  let {
    row,
    name,
    showName,
    active,
    playable,
    onseek,
    oncorrect,
  }: {
    row: Segment;
    name: string;
    showName: boolean;
    active: boolean;
    playable: boolean;
    onseek: (ms: number) => void;
    oncorrect?: (row: Segment) => void;
  } = $props();
</script>

<article
  class={['turn', active && 'active', !showName && 'continued', oncorrect && 'has-options']}
  id={row.id}
  tabindex="-1"
>
  {#if playable}
    <button
      class="time"
      onclick={() => onseek(row.start_ms)}
      aria-label={`Play from ${timestamp(row.start_ms)}`}
    >
      {timestamp(row.start_ms)}
    </button>
  {:else}
    <span class="time">{timestamp(row.start_ms)}</span>
  {/if}
  {#if showName}<strong>{name}</strong>{/if}
  <p>{row.text}</p>
  {#if oncorrect}
    <SpeakerSplitMenu
      label={`Speaker options at ${timestamp(row.start_ms)}`}
      class="turn-menu-trigger"
      notSpeaker={name}
      onsplit={() => oncorrect?.(row)}
    />
  {/if}
</article>
