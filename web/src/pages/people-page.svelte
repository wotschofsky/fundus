<script lang="ts">
  import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { Plus } from '@lucide/svelte';
  import { api, peopleQuery, send, type Person } from '../api';
  import ConfirmDialog, { type ConfirmRequest } from '../components/confirm-dialog.svelte';
  import EmptyState from '../components/empty-state.svelte';
  import ErrorNotice from '../components/error-notice.svelte';
  import LoadingState from '../components/loading-state.svelte';
  import PersonRow from '../components/person-row.svelte';

  const qc = useQueryClient();
  let confirming: ConfirmRequest | null = $state(null);
  let name = $state('');
  const people = createQuery(() => peopleQuery);
  const add = createMutation(() => ({
    mutationFn: () => api<Person>('/v1/people', send('POST', { name: name.trim() })),
    onSuccess: () => {
      name = '';
      return qc.invalidateQueries({ queryKey: ['people'] });
    },
  }));
</script>

<header class="page-header">
  <h1>People</h1>
</header>
<p class="helper">
  Confirm who is speaking in a transcript to build a voice profile. Future recordings then recognize
  them automatically; uncertain matches stay unidentified.
</p>
<form
  class="inline-form"
  onsubmit={(e) => {
    e.preventDefault();
    if (name.trim()) add.mutate();
  }}
>
  <label class="sr-only" for="person-name">Person’s name</label>
  <input id="person-name" placeholder="Add someone by name" maxlength={120} bind:value={name} />
  <button class="primary" disabled={add.isPending || !name.trim()}>
    <Plus size={16} aria-hidden="true" />
    Add person
  </button>
</form>
<ErrorNotice error={add.error ?? people.error} />
{#if people.isPending}
  <LoadingState />
{:else if people.data?.people.length === 0}
  <EmptyState
    title="No people yet"
    text="Add someone here, or name a speaker directly in a transcript."
  />
{:else}
  <div class="rows">
    {#each people.data?.people ?? [] as p (p.id)}
      <PersonRow person={p} confirm={(request) => (confirming = request)} />
    {/each}
  </div>
{/if}
<ConfirmDialog bind:request={confirming} />
