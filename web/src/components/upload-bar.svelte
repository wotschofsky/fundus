<script lang="ts">
  import { uploads } from '../uploads.svelte';

  const percent = $derived(uploads.total ? Math.floor((uploads.sent / uploads.total) * 100) : 0);
  const of = $derived(uploads.count > 1 ? ` (${uploads.index} of ${uploads.count})` : '');

  /** Leaving mid-upload would drop it; the browser asks first. */
  function onbeforeunload(event: BeforeUnloadEvent) {
    if (uploads.total) event.preventDefault();
  }
</script>

<svelte:window {onbeforeunload} />

{#if uploads.total || uploads.problems.length}
  <div class="upload-bar">
    {#if uploads.total}
      <progress max={uploads.total} value={uploads.sent} aria-label="Upload progress"></progress>
    {/if}
    <div class="upload-row">
      <div role="status">
        {#if uploads.total}
          <span class="upload-name">
            {uploads.checking ? 'Checking the upload…' : `Uploading ${uploads.name}${of}`}
          </span>
        {/if}
        {#each uploads.problems as problem, i (i)}
          <span class="upload-problem">{problem}</span>
        {/each}
      </div>
      {#if uploads.total}
        <span aria-hidden="true">{percent}%</span>
      {:else}
        <button class="text-button" onclick={() => (uploads.problems = [])}>Dismiss</button>
      {/if}
    </div>
  </div>
{/if}
