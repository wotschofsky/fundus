<script lang="ts">
  import { onMount } from 'svelte';
  import { QueryClient, createQuery, setQueryClientContext } from '@tanstack/svelte-query';
  import { api, ApiError, type Me } from './api';
  import { router } from './router.svelte';
  import EmptyState from './components/empty-state.svelte';
  import ErrorNotice from './components/error-notice.svelte';
  import LoadingState from './components/loading-state.svelte';
  import AuthorizePage from './pages/authorize-page.svelte';
  import LoginPage from './pages/login-page.svelte';
  import PeoplePage from './pages/people-page.svelte';
  import RecordingPage from './pages/recording-page.svelte';
  import RecordingsPage from './pages/recordings-page.svelte';
  import SearchPage from './pages/search-page.svelte';
  import SettingsPage from './pages/settings-page.svelte';
  import TrashPage from './pages/trash-page.svelte';

  const client = new QueryClient({
    defaultOptions: {
      queries: {
        retry: (count, error) => !(error instanceof ApiError && error.status < 500) && count < 2,
        staleTime: 10_000,
      },
    },
  });
  setQueryClientContext(client);
  onMount(() => {
    client.mount();
    return () => client.unmount();
  });

  const me = createQuery(() => ({
    queryKey: ['me'],
    queryFn: () => api<Me>('/v1/me'),
    retry: false,
  }));
  const path = $derived(router.url.pathname);
  const entryId = $derived(path.match(/^\/entries\/([^/]+)\/?$/)?.[1]);
  const nav = [
    ['/', 'Recordings'],
    ['/search', 'Search'],
    ['/people', 'People'],
    ['/settings', 'Settings'],
  ];
  const current = (to: string) => (to === '/' ? path === '/' : path.startsWith(to));

  /** "/" jumps to search from anywhere. */
  function onkeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement;
    if (event.key !== '/' || event.metaKey || event.ctrlKey) return;
    if (target.closest('input, textarea, select, [contenteditable]')) return;
    event.preventDefault();
    const input = document.getElementById('search-input');
    if (input) input.focus();
    else router.navigate('/search');
  }
</script>

<svelte:window {onkeydown} />

{#if me.isPending}
  <LoadingState label="Opening your vault…" />
{:else if me.error instanceof ApiError && me.error.status === 401}
  <LoginPage />
{:else if !me.data}
  <main class="login">
    <ErrorNotice error={me.error} />
    <button onclick={() => me.refetch()}>Try again</button>
  </main>
{:else if path === '/oauth/authorize'}
  <AuthorizePage />
{:else}
  <div class="shell">
    <header class="app-header">
      <a class="brand" href="/">Vault</a>
      <nav aria-label="Main">
        {#each nav as [to, label] (to)}
          <a
            href={to}
            class={{ active: current(to) }}
            aria-current={current(to) ? 'page' : undefined}>{label}</a
          >
        {/each}
      </nav>
    </header>
    <main class="workspace">
      {#if path === '/'}
        <RecordingsPage />
      {:else if path === '/trash'}
        <TrashPage />
      {:else if entryId}
        {#key entryId}
          <RecordingPage id={decodeURIComponent(entryId)} />
        {/key}
      {:else if path === '/search'}
        <SearchPage />
      {:else if path === '/people'}
        <PeoplePage />
      {:else if path === '/settings'}
        <SettingsPage me={me.data} />
      {:else}
        <EmptyState title="Page not found" text="Return to your recordings to find what you need.">
          <a class="button" href="/">Recordings</a>
        </EmptyState>
      {/if}
    </main>
  </div>
{/if}
