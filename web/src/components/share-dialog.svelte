<!-- Sharing a recording, laid out like a document's share dialog: who can open it, when the link
     stops working, and Copy link. Choosing "Anyone with the link" makes the link (for 30 days);
     going back to "Only you" asks first, since the link then stops working for good. -->
<script lang="ts">
  import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { Check, ChevronDown, Globe, Link, Lock } from '@lucide/svelte';
  import { api, send, shareQuery, type Expiry, type Share } from '../api';
  import { date } from '../format';
  import ErrorNotice from './error-notice.svelte';

  let {
    id,
    title,
    open = $bindable(false),
  }: { id: string; title: string; open: boolean } = $props();
  const qc = useQueryClient();
  let dialog: HTMLDialogElement;
  let done: HTMLButtonElement;
  /** "Only you" was chosen while shared: the dialog asks before the link stops working. */
  let stopping = $state(false);
  let copied = $state<'idle' | 'copied' | 'failed'>('idle');
  const expiries: [Expiry, string][] = [
    ['1d', 'Expires in 1 day'],
    ['7d', 'Expires in 7 days'],
    ['30d', 'Expires in 30 days'],
    ['never', 'Never expires'],
  ];

  const share = createQuery(() => shareQuery(id));
  const link = $derived(share.data ?? null);
  const saved = (value: Share | null) => {
    qc.setQueryData(['share', id], value);
    return qc.invalidateQueries({ queryKey: ['shares'] });
  };
  const save = createMutation(() => ({
    mutationFn: (expires: Expiry) =>
      api<Share>(`/v1/entries/${id}/share`, send('PUT', { expires })),
    onSuccess: saved,
  }));
  const stop = createMutation(() => ({
    mutationFn: () => api(`/v1/entries/${id}/share`, send('DELETE')),
    onSuccess: () => {
      stopping = false;
      return saved(null);
    },
  }));
  /** What the access menu shows: the saved state, or a choice still being saved or confirmed. */
  const access = $derived(stopping ? 'private' : link || save.isPending ? 'link' : 'private');
  const busy = $derived(share.isPending || save.isPending || stop.isPending);

  $effect(() => {
    if (open && !dialog.open) {
      stopping = false;
      copied = 'idle';
      save.reset();
      stop.reset();
      dialog.showModal();
      // Done, the safe default, rather than the access menu the dialog would otherwise focus.
      done.focus();
    }
    if (!open && dialog.open) dialog.close();
  });
  $effect(() => {
    if (copied !== 'copied') return;
    const timer = setTimeout(() => (copied = 'idle'), 1800);
    return () => clearTimeout(timer);
  });

  function choose(value: string) {
    if (value === 'link' && !link) save.mutate('30d');
    if (value === 'private' && link) stopping = true;
  }
  function copy() {
    if (!link) return;
    // No clipboard API outside secure origins (plain HTTP on a LAN address): show the link.
    (navigator.clipboard?.writeText(link.url) ?? Promise.reject()).then(
      () => (copied = 'copied'),
      () => (copied = 'failed'),
    );
  }
</script>

<dialog
  bind:this={dialog}
  class="confirm share-dialog"
  aria-labelledby="share-title"
  oncancel={(event) => {
    event.preventDefault();
    if (!stop.isPending) open = false;
  }}
>
  <h2 id="share-title">Share “{title}”</h2>
  <h3>General access</h3>
  <div class={['access', access === 'link' && 'shared']}>
    <span class="access-icon" aria-hidden="true">
      {#if access === 'link'}<Globe size={16} />{:else}<Lock size={16} />{/if}
    </span>
    <div class="access-body">
      <span class="quiet-select">
        <select
          aria-label="Who can open this recording"
          value={access}
          disabled={busy || stopping}
          onchange={(event) => choose(event.currentTarget.value)}
        >
          <option value="private">Only you</option>
          <option value="link">Anyone with the link</option>
        </select>
        <ChevronDown size={14} aria-hidden="true" />
      </span>
      <p>
        {#if stopping}
          Anyone who has the link loses access right away. Sharing again makes a new link.
        {:else if access === 'link'}
          Anyone with the link can play it and read its summary and transcript, without signing in.
        {:else}
          Only you can open it, signed in to your library.
        {/if}
      </p>
      {#if stopping}
        <div class="stop-actions">
          <button type="button" disabled={stop.isPending} onclick={() => (stopping = false)}>
            Keep sharing
          </button>
          <button
            type="button"
            class="danger"
            disabled={stop.isPending}
            onclick={() => stop.mutate()}
          >
            {stop.isPending ? 'Stopping…' : 'Stop sharing'}
          </button>
        </div>
      {/if}
    </div>
    {#if link && !stopping}
      <!-- Rebuilt per expiry, so it shows the saved date rather than the option just picked. -->
      {#key link.expires_at}
        <span class="quiet-select expiry">
          <select
            aria-label="When the link stops working"
            value={link.expires_at ? 'current' : 'never'}
            disabled={busy}
            onchange={(event) => {
              const value = event.currentTarget.value;
              if (value !== 'current') save.mutate(value as Expiry);
            }}
          >
            <!-- The saved date; choosing it again changes nothing. -->
            {#if link.expires_at}
              <option value="current">Expires {date(link.expires_at)}</option>
            {/if}
            {#each expiries as [value, label] (value)}<option {value}>{label}</option>{/each}
          </select>
          <ChevronDown size={14} aria-hidden="true" />
        </span>
      {/key}
    {/if}
  </div>
  {#if copied === 'failed' && link}
    <label class="share-url">
      Copying is unavailable here. Select the link and copy it.
      <input
        readonly
        value={link.url}
        onfocus={(event) => event.currentTarget.select()}
        {@attach (input) => input.focus()}
      />
    </label>
  {/if}
  <ErrorNotice error={share.error ?? save.error ?? stop.error} />
  <div class="dialog-actions share-actions">
    <button type="button" disabled={!link || stopping} onclick={copy}>
      {#if copied === 'copied'}
        <Check size={15} aria-hidden="true" />
        Link copied
      {:else}
        <Link size={15} aria-hidden="true" />
        Copy link
      {/if}
    </button>
    <span class="sr-only" role="status">{copied === 'copied' ? 'Link copied' : ''}</span>
    <button
      bind:this={done}
      type="button"
      class="primary"
      disabled={stop.isPending}
      onclick={() => (open = false)}
    >
      Done
    </button>
  </div>
</dialog>
