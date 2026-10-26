<!-- Wraps the query's words in <mark>, case-insensitively. -->
<script lang="ts">
  let { text, query }: { text: string; query: string } = $props();

  const parts = $derived.by(() => {
    const words = query
      .split(/[^\p{L}\p{N}]+/u)
      .filter((w) => w.length > 1)
      .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
    return words.length ? text.split(new RegExp(`(${words.join('|')})`, 'giu')) : [text];
  });
</script>

{#each parts as part, i}{#if i % 2}<mark>{part}</mark>{:else}{part}{/if}{/each}
