<!-- One person; destructive actions go through the page's shared confirm dialog. -->
<script lang="ts">
  import { createMutation, useQueryClient } from '@tanstack/svelte-query';
  import { api, send, type Person } from '../api';
  import type { ConfirmRequest } from './confirm-dialog.svelte';
  import ErrorNotice from './error-notice.svelte';

  let { person, confirm }: { person: Person; confirm: (request: ConfirmRequest) => void } =
    $props();
  const qc = useQueryClient();
  let renaming = $state(false);
  let name = $state('');
  // Names also appear in transcripts and summaries.
  const refresh = () =>
    Promise.all(
      ['people', 'entry', 'entries'].map((key) => qc.invalidateQueries({ queryKey: [key] })),
    );
  const rename = createMutation(() => ({
    mutationFn: () => api(`/v1/people/${person.id}`, send('PATCH', { name: name.trim() })),
    onSuccess: async () => {
      await refresh();
      renaming = false;
    },
  }));
</script>

<div class="person">
  {#if renaming}
    <form
      class="inline-form"
      onsubmit={(e) => {
        e.preventDefault();
        if (name.trim()) rename.mutate();
      }}
    >
      <label class="sr-only" for={`rename-${person.id}`}>Name</label>
      <input
        id={`rename-${person.id}`}
        {@attach (input) => input.focus()}
        maxlength={120}
        bind:value={name}
        onkeydown={(e) => e.key === 'Escape' && (renaming = false)}
      />
      <button class="primary" disabled={rename.isPending || !name.trim()}>Save</button>
      <button type="button" onclick={() => (renaming = false)}>Cancel</button>
    </form>
  {:else}
    <div class="person-body">
      <h2>{person.name}</h2>
      <p>
        {person.samples === 0
          ? 'No voice profile yet'
          : `${person.samples} confirmed voice sample${person.samples !== 1 ? 's' : ''}`}
      </p>
    </div>
    <div class="row-actions">
      <button
        onclick={() => {
          name = person.name;
          renaming = true;
        }}
      >
        Rename
      </button>
      {#if person.samples > 0}
        <button
          onclick={() =>
            confirm({
              title: `Remove ${person.name}’s voice profile?`,
              text: 'Their name and confirmed transcript assignments stay. New recordings will not recognize them until you confirm their voice again.',
              action: 'Remove voice profile',
              danger: true,
              run: () => api(`/v1/people/${person.id}/voice`, send('DELETE')).then(refresh),
            })}
        >
          Remove voice
        </button>
      {/if}
      <button
        class="danger"
        onclick={() =>
          confirm({
            title: `Delete ${person.name}?`,
            text: 'Their voice profile is deleted and their speakers become unidentified. Transcript text is kept.',
            action: 'Delete person',
            danger: true,
            run: () => api(`/v1/people/${person.id}`, send('DELETE')).then(refresh),
          })}
      >
        Delete
      </button>
    </div>
  {/if}
  <ErrorNotice error={rename.error} />
</div>
