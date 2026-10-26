<!-- Processing state for a library row; nothing once the recording is ready. -->
<script lang="ts">
  import { LoaderCircle } from '@lucide/svelte';
  import { isProcessing, type Entry } from '../api';
  import { stageName } from '../format';

  let { entry }: { entry: Entry } = $props();
</script>

{#if entry.stage !== 'complete'}
  <span class={['stage', entry.stage === 'failed' && 'failed']}
    >{#if isProcessing(entry)}<LoaderCircle
        class="spin"
        size={12}
        aria-hidden="true"
      />{/if}{stageName(entry)}</span
  >
{/if}
