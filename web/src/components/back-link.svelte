<!-- Back to the page you came from (search, a filtered list), or to Recordings from a fresh tab. -->
<script lang="ts">
  import { ArrowLeft } from '@lucide/svelte';
  import { router } from '../router.svelte';

  function back(event: MouseEvent) {
    if (!router.inApp) return;
    event.preventDefault();
    history.back();
  }
</script>

<a class="back" href="/" onclick={back}>
  <ArrowLeft size={15} aria-hidden="true" />
  {router.inApp ? 'Back' : 'Recordings'}
</a>
