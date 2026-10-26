<script lang="ts">
  import { LoaderCircle } from '@lucide/svelte';

  let { label = 'Loading…' }: { label?: string } = $props();
</script>

<div class="loading" role="status">
  <LoaderCircle class="spin" size={18} aria-hidden="true" />
  {label}
</div>
