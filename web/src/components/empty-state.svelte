<script lang="ts">
  import type { Snippet } from 'svelte';

  let { title, text, children }: { title: string; text: string; children?: Snippet } = $props();
</script>

<section class="empty">
  <h2>{title}</h2>
  <p>{text}</p>
  {@render children?.()}
</section>
