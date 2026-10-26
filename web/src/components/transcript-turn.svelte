<!-- One transcript segment. -->
<script lang="ts">
  import type { Segment } from '../api';
  import { timestamp } from '../format';

  let {
    row,
    name,
    showName,
    active,
    playable,
    onseek,
  }: {
    row: Segment;
    name: string;
    showName: boolean;
    active: boolean;
    playable: boolean;
    onseek: (ms: number) => void;
  } = $props();
</script>

<article class={['turn', active && 'active', !showName && 'continued']} id={row.id}>
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
  <div>
    {#if showName}<strong>{name}</strong>{/if}
    <p>{row.text}</p>
  </div>
</article>
