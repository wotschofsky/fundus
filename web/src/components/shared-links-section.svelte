<!-- Every recording shared by link, so a link that never expires is not forgotten. -->
<script lang="ts">
  import { createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { api, gone, linkToken, send, type SharedLink, sharesQuery } from '../api';
  import { LinkClock, forget } from '../link-clock.svelte';
  import { date, dateTime } from '../format';
  import ConfirmDialog, { type ConfirmRequest } from './confirm-dialog.svelte';
  import ErrorNotice from './error-notice.svelte';

  const qc = useQueryClient();
  let confirming: ConfirmRequest | null = $state(null);
  const shares = createQuery(() => sharesQuery);
  const clock = new LinkClock(
    () => shares.data?.shares.map((link) => link.expires_at) ?? [],
    () => shares.refetch({ cancelRefetch: false }),
  );
  /** The links that work by this page's clock: one drops out of the list as it expires. */
  const working = $derived(shares.data?.shares.filter((link) => clock.offered(link)) ?? []);
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
        // Stopped, here or before: it leaves the list now, even if the refresh below fails.
        forget(link.url);
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
    {#if working.length}
      <div class="rows">
        {#each working as link (link.entry_id)}
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
