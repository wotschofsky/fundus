<script lang="ts">
  import { Check, Copy } from '@lucide/svelte';

  let { value, label }: { value: string; label: string } = $props();
  let result: 'idle' | 'copied' | 'failed' = $state('idle');

  $effect(() => {
    if (result === 'idle') return;
    const timer = setTimeout(() => (result = 'idle'), 1800);
    return () => clearTimeout(timer);
  });

  function copy() {
    // No clipboard API outside secure origins (plain HTTP on a LAN address).
    (navigator.clipboard?.writeText(value) ?? Promise.reject()).then(
      () => (result = 'copied'),
      () => (result = 'failed'),
    );
  }
</script>

<div class="copy-field">
  <code>{value}</code>
  <button
    type="button"
    class="icon-button"
    aria-label={result === 'copied' ? 'Copied' : label}
    title={result === 'failed' ? 'Copy is unavailable here. Select the text instead.' : label}
    onclick={copy}
  >
    {#if result === 'copied'}<Check size={16} />{:else}<Copy size={16} />{/if}
  </button>
</div>
