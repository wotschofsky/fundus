<!-- Every recording shared by link, so a link that never expires is not forgotten. -->
<script lang="ts">
  import { createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { api, gone, linkToken, send, type SharedLink } from '../api';
  import { date, dateTime } from '../format';
  import ConfirmDialog, { type ConfirmRequest } from './confirm-dialog.svelte';
  import ErrorNotice from './error-notice.svelte';

  const qc = useQueryClient();
  let confirming: ConfirmRequest | null = $state(null);
  const shares = createQuery(() => ({
    queryKey: ['shares'],
    queryFn: () => api<{ shares: SharedLink[] }>('/v1/shares'),
  }));
  const stop = (link: SharedLink) =>
    (confirming = {
      title: `Stop sharing “${link.title}”?`,
      text: 'The link stops working right away, also for anyone who already has it. Sharing again makes a new link.',
      action: 'Stop sharing',
      danger: true,
      run: async () => {
        const named = `token=${encodeURIComponent(linkToken(link))}`;
        // Already gone (stopped, or replaced from another tab): the list shows what is left.
        await api(`/v1/entries/${link.entry_id}/share?${named}`, send('DELETE')).catch((error) => {
          if (!gone(error)) throw error;
        });
        qc.removeQueries({ queryKey: ['share', link.entry_id] });
        await qc.invalidateQueries({ queryKey: ['shares'] });
      },
    });
</script>

<section class="settings-section" id="shared">
  <h2>Shared links</h2>
  <div class="setting">
    <p>
      Recordings anyone with their link can play and read, without signing in. Share a recording
      from its page.
    </p>
    <ErrorNotice error={shares.error} />
    {#if shares.data?.shares.length}
      <div class="rows">
        {#each shares.data.shares as link (link.entry_id)}
          <div class="connection">
            <div>
              <a href={`/entries/${encodeURIComponent(link.entry_id)}`}
                ><strong>{link.title}</strong></a
              >
              <small>
                Shared {date(link.created_at)} · {link.expires_at
                  ? `Expires ${dateTime(link.expires_at)}`
                  : 'Never expires'}
              </small>
            </div>
            <button class="danger" onclick={() => stop(link)}>Stop sharing</button>
          </div>
        {/each}
      </div>
    {:else if shares.data}
      <p class="muted">No recordings are shared.</p>
    {/if}
  </div>
</section>
<ConfirmDialog bind:request={confirming} />
