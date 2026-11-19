<!-- Sharing a recording, laid out like a document's share dialog: who can open it, when the link
     stops working, and Copy link. Choosing "Anyone with the link" makes the link (for 30 days);
     going back to "Only you" asks first, since the link then stops working for good. -->
<script lang="ts">
  import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { Check, ChevronDown, Globe, Link, Lock } from '@lucide/svelte';
  import {
    api,
    type Expiry,
    gone,
    keepAnswer,
    linkToken,
    send,
    sentAt,
    type Share,
    shareQuery,
  } from '../api';
  import { date } from '../format';
  import { forget, type LinkClock } from '../link-clock.svelte';
  import ErrorNotice from './error-notice.svelte';

  let {
    id,
    title,
    clock,
    open = $bindable(false),
  }: {
    id: string;
    title: string;
    /** The recording page's clock, which tracks this link's expiry for both. */
    clock: LinkClock;
    open: boolean;
  } = $props();
  const qc = useQueryClient();
  let dialog: HTMLDialogElement;
  let done: HTMLButtonElement;
  /** "Only you" was chosen while shared: the URL of the link the dialog asks about stopping. */
  let stopping = $state<string | null>(null);
  let copied = $state<'idle' | 'copied' | 'failed'>('idle');
  const expiries: [Expiry, string][] = [
    ['1d', 'Expires in 1 day'],
    ['7d', 'Expires in 7 days'],
    ['30d', 'Expires in 30 days'],
    ['never', 'Never expires'],
  ];

  const share = createQuery(() => shareQuery(id));
  /** The saved link while it works by this page's clock: never offered once it has expired, or
   * once the server has said it is gone. */
  const link = $derived(clock.offered(share.data));
  /** When a change goes out, what the page then held of the link, for `keepAnswer`. */
  const sending = () => ({ sent: sentAt() });
  const save = createMutation(() => ({
    onMutate: sending,
    mutationFn: (expires: Expiry) =>
      api<Share>(`/v1/entries/${id}/share`, send('PUT', { expires })),
    onSuccess: (value, _, context) => keepAnswer(qc, id, value, context.sent),
  }));
  /** The named link is no longer there (stopped, expired or replaced elsewhere): drop it and ask
   * again. */
  const reread = (error: Error, url: string) => {
    if (!gone(error)) return;
    forget(url);
    qc.invalidateQueries({ queryKey: ['shares'] });
    return qc.invalidateQueries({ queryKey: ['share', id] });
  };
  /** Changes only the link shown, so it never shares again or touches a link that replaced it. */
  const extend = createMutation(() => ({
    onMutate: sending,
    mutationFn: ({ link, expires }: { link: Share; expires: Expiry }) =>
      api<Share>(`/v1/entries/${id}/share`, send('PATCH', { expires, token: linkToken(link) })),
    onSuccess: (value, _, context) => keepAnswer(qc, id, value, context.sent),
    onError: (error, { link }) => reread(error, link.url),
  }));
  const stop = createMutation(() => ({
    onMutate: sending,
    mutationFn: (url: string) =>
      api(
        `/v1/entries/${id}/share?token=${encodeURIComponent(linkToken({ url }))}`,
        send('DELETE'),
      ),
    onSuccess: (_, url, context) => {
      stopping = null;
      // Gone for every reader, Settings' list too, even before it refreshes.
      forget(url);
      return keepAnswer(qc, id, null, context.sent);
    },
    onError: (error, url) => {
      stopping = null;
      return reread(error, url);
    },
  }));
  /** Stop sharing is being confirmed, for the link still offered. */
  const confirming = $derived(!!link && stopping === link.url);
  /** What the access menu shows: the saved state, or a choice still being saved or confirmed. */
  const access = $derived(confirming ? 'private' : link || save.isPending ? 'link' : 'private');
  /** A change is being saved: the dialog stays open until it lands, so reopening it cannot drop
   * one change's result while another (such as Stop sharing) goes ahead. */
  const saving = $derived(save.isPending || extend.isPending || stop.isPending);
  const busy = $derived(share.isPending || saving);

  $effect(() => {
    if (open && !dialog.open) {
      stopping = null;
      copied = 'idle';
      save.reset();
      extend.reset();
      stop.reset();
      // What the page has may be out of date: by its clock now, and from the server again (which
      // the clock has just done if the link lapsed).
      if (!clock.check()) share.refetch();
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
    if (value === 'link' && !link) {
      stopping = null;
      save.mutate('30d');
    }
    if (value === 'private' && link) stopping = link.url;
  }
  function copy() {
    clock.check();
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
    if (!saving) open = false;
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
          disabled={busy || confirming}
          onchange={(event) => choose(event.currentTarget.value)}
        >
          <option value="private">Only you</option>
          <option value="link">Anyone with the link</option>
        </select>
        <ChevronDown size={14} aria-hidden="true" />
      </span>
      <p>
        {#if confirming}
          Anyone who has the link loses access right away. Sharing again makes a new link.
        {:else if access === 'link'}
          Anyone with the link can play it and read its summary and transcript, without signing in.
        {:else}
          Only you can open it, signed in to your library.
        {/if}
      </p>
      {#if confirming && stopping}
        {@const url = stopping}
        <div class="stop-actions">
          <button type="button" disabled={stop.isPending} onclick={() => (stopping = null)}>
            Keep sharing
          </button>
          <button
            type="button"
            class="danger"
            disabled={stop.isPending}
            onclick={() => stop.mutate(url)}
          >
            {stop.isPending ? 'Stopping…' : 'Stop sharing'}
          </button>
        </div>
      {/if}
    </div>
    {#if link && !confirming}
      <!-- Rebuilt when the expiry is saved or fails to save, so it always shows the saved one. -->
      {#key `${link.expires_at} ${extend.status}`}
        <span class="quiet-select expiry">
          <select
            aria-label="When the link stops working"
            value={link.expires_at ? 'current' : 'never'}
            disabled={busy}
            onchange={(event) => {
              const value = event.currentTarget.value;
              if (value !== 'current') extend.mutate({ link, expires: value as Expiry });
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
        oncopy={(event) => {
          // Copied by hand: only while the link still works.
          clock.check();
          if (!link) event.preventDefault();
        }}
        {@attach (input) => input.focus()}
      />
    </label>
  {/if}
  <!-- A link gone meanwhile is not an error: the dialog asks again and shows what is there. -->
  <ErrorNotice
    error={share.error ??
      save.error ??
      (gone(extend.error) ? null : extend.error) ??
      (gone(stop.error) ? null : stop.error)}
  />
  <div class="dialog-actions share-actions">
    <button type="button" disabled={!link || confirming} onclick={copy}>
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
      disabled={saving}
      onclick={() => (open = false)}
    >
      Done
    </button>
  </div>
</dialog>
