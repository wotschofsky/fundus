<!-- The root on a share link (`/s/{token}`): no sign-in and none of the library, only the
     recording. main.ts loads it instead of the app, so no owner code runs here. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { QueryClient, setQueryClientContext } from '@tanstack/svelte-query';
  import { ApiError } from './api';
  import SharePage from './pages/share-page.svelte';

  const client = new QueryClient({
    defaultOptions: {
      queries: {
        retry: (count, error) => !(error instanceof ApiError && error.status < 500) && count < 2,
      },
    },
  });
  setQueryClientContext(client);
  onMount(() => {
    client.mount();
    return () => client.unmount();
  });
  // The server serves this page at `/s/{token}` only.
  // Decoded once, as the server reads it, so a link with an escaped character still opens.
  const segment = location.pathname.split('/')[2] ?? '';
  const token = (() => {
    try {
      return decodeURIComponent(segment);
    } catch {
      return segment;
    }
  })();
</script>

<header class="app-header">
  <span class="brand">Fundus</span>
  <span class="header-note">Shared recording</span>
</header>
<main class="workspace">
  <SharePage {token} />
</main>
