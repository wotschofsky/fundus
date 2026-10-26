<script lang="ts">
  import { useQueryClient } from '@tanstack/svelte-query';
  import { RotateCcw } from '@lucide/svelte';
  import { api, send } from '../api';
  import { createEntries } from '../entries';
  import { date, duration } from '../format';
  import BackLink from '../components/back-link.svelte';
  import ConfirmDialog, { type ConfirmRequest } from '../components/confirm-dialog.svelte';
  import EmptyState from '../components/empty-state.svelte';
  import ErrorNotice from '../components/error-notice.svelte';
  import LoadMoreButton from '../components/load-more-button.svelte';
  import LoadingState from '../components/loading-state.svelte';

  const qc = useQueryClient();
  const entries = createEntries(() => '&trash=true', false);
  let confirming: ConfirmRequest | null = $state(null);
  let error: unknown = $state(null);
  const all = $derived(entries.data ?? []);
  // Labels count only recordings outside Trash, and search shows restored ones again.
  const refresh = () =>
    Promise.all(
      ['entries', 'labels', 'search'].map((key) => qc.invalidateQueries({ queryKey: [key] })),
    );
  const restore = (id: string) =>
    api(`/v1/entries/${id}/restore`, send('POST')).then(
      () => {
        error = null;
        return refresh();
      },
      (e) => (error = e),
    );
</script>

<BackLink />
<header class="page-header">
  <h1>Trash</h1>
</header>
<p class="helper">
  Recordings in Trash are hidden from the library, search and chat apps. Restore them at any time;
  deleting permanently removes their audio and transcript from your server.
</p>
<ErrorNotice error={entries.error ?? error} />
{#if entries.isPending}
  <LoadingState />
{:else if all.length === 0}
  <EmptyState title="Trash is empty" text="Recordings you move to Trash appear here." />
{:else}
  <div class="rows">
    {#each all as e (e.id)}
      <div class="trash-row">
        <div>
          <div class="recording-meta">
            <span>{date(e.recorded_at)}</span>
            <span>{duration(e.duration_ms)}</span>
          </div>
          <h3>{e.title}</h3>
        </div>
        <div class="row-actions">
          <button onclick={() => restore(e.id)}>
            <RotateCcw size={14} aria-hidden="true" />
            Restore
          </button>
          <button
            class="danger"
            onclick={() =>
              (confirming = {
                title: `Delete “${e.title}” permanently?`,
                text: 'Its archived audio, transcript and contributed voice samples are removed from your server. This cannot be undone. Originals on your recorder are not affected.',
                action: 'Delete permanently',
                danger: true,
                run: () => api(`/v1/entries/${e.id}`, send('DELETE')).then(refresh),
              })}
          >
            Delete permanently
          </button>
        </div>
      </div>
    {/each}
  </div>
{/if}
<LoadMoreButton query={entries} />
<ConfirmDialog bind:request={confirming} />
