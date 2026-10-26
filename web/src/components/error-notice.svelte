<script lang="ts">
  let { error }: { error: unknown } = $props();
</script>

{#if error}
  <div class="notice error" role="alert">
    {error instanceof Error ? error.message : 'Something went wrong. Please try again.'}
  </div>
{/if}
