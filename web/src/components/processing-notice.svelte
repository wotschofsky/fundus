<!-- Status of an unfinished recording, with Pause while it runs and Resume/Retry once stopped. -->
<script lang="ts">
  import { LoaderCircle, Pause, Play } from '@lucide/svelte';
  import { isProcessing, type Entry } from '../api';
  import { stageName } from '../format';

  let {
    entry,
    busy,
    onpause,
    onresume,
  }: { entry: Entry; busy: boolean; onpause: () => void; onresume: () => void } = $props();
  const processing = $derived(isProcessing(entry));
</script>

{#if entry.stage !== 'complete'}
  <div class={['notice status', entry.stage === 'failed' && 'error']} role="status">
    <div>
      <strong
        >{#if processing}<LoaderCircle class="spin" size={14} aria-hidden="true" />{/if}{stageName(
          entry,
        )}</strong
      >
      <p>
        {entry.failure ??
          (processing
            ? 'Your server is processing this recording. You can leave this page.'
            : 'Processing is paused. Resume to continue where it stopped.')}
      </p>
    </div>
    {#if processing}
      <button disabled={busy || entry.pause_requested} onclick={onpause}>
        <Pause size={14} aria-hidden="true" />
        Pause
      </button>
    {:else}
      <button disabled={busy} onclick={onresume}>
        <Play size={14} aria-hidden="true" />
        {entry.stage === 'failed' ? 'Retry' : 'Resume'}
      </button>
    {/if}
  </div>
{/if}
