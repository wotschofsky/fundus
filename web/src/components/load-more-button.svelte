<script lang="ts">
  import { ChevronDown } from '@lucide/svelte';

  let {
    query,
  }: {
    query: { hasNextPage: boolean; isFetchingNextPage: boolean; fetchNextPage: () => unknown };
  } = $props();
</script>

{#if query.hasNextPage}
  <button
    class="load-more"
    disabled={query.isFetchingNextPage}
    onclick={() => query.fetchNextPage()}
  >
    {query.isFetchingNextPage ? 'Loading…' : 'Show older recordings'}
    <ChevronDown size={16} aria-hidden="true" />
  </button>
{/if}
