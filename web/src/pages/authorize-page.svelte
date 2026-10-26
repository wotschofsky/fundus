<script lang="ts">
  import { createMutation, createQuery } from '@tanstack/svelte-query';
  import { api, send, type Authorization } from '../api';
  import ErrorNotice from '../components/error-notice.svelte';
  import LoadingState from '../components/loading-state.svelte';

  const search = location.search;
  const request = createQuery(() => ({
    queryKey: ['oauth-authorization', search],
    queryFn: () => api<Authorization>(`/v1/oauth/authorize${search}`),
    retry: false,
    refetchOnWindowFocus: false,
  }));
  const consent = createMutation(() => ({
    mutationFn: (approve: boolean) =>
      api<{ redirect_url: string }>(
        '/v1/oauth/authorize',
        send('POST', { request_id: request.data!.request_id, approve }),
      ),
    onSuccess: (value) => location.assign(value.redirect_url),
  }));
  const data = $derived(request.data);
  const owner = $derived(data?.scope === 'vault:owner');
  const done = $derived(consent.isPending || consent.isSuccess);
</script>

{#if request.isPending}
  <LoadingState />
{:else}
  <main class="login oauth-consent">
    <h1>Connect to your vault</h1>
    {#if data}
      <p>
        <strong>{data.client_name}</strong> is requesting
        {owner ? 'full access to your vault' : 'read access to your recordings'}.
      </p>
      <p>
        {owner
          ? 'It will be able to upload recordings, play and read your archive, edit transcripts, speakers, people and labels, and delete recordings.'
          : 'It will be able to search recordings and read transcripts, summaries, labels and speaker names. It cannot upload, edit or delete anything.'}
      </p>
      <div class="oauth-destination">
        <span>Returns to</span>
        <code>{data.redirect_uri}</code>
      </div>
      <p class="oauth-hint">
        Approve only if you just started connecting this app. Its name is supplied by the app
        itself.
      </p>
    {/if}
    <ErrorNotice error={request.error ?? consent.error} />
    {#if data}
      <div class="oauth-actions">
        <button disabled={done} onclick={() => consent.mutate(false)}>Cancel</button>
        <button class="primary" disabled={done} onclick={() => consent.mutate(true)}>
          {consent.isPending ? 'Connecting…' : owner ? 'Allow full access' : 'Allow read access'}
        </button>
      </div>
    {/if}
    <small>You can disconnect this app at any time in Settings.</small>
  </main>
{/if}
