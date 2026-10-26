<!-- Who a diarized speaker is: pick a person, add one, confirm a voice match or a self-introduction. -->
<script lang="ts">
  import type { Person, Speaker } from '../api';
  import ErrorNotice from './error-notice.svelte';

  /** Select value for the "New person…" option; person ids are UUIDs, so it cannot collide. */
  const NEW_PERSON = '+new';

  const STATES: Record<Speaker['state'], string> = {
    automatic: 'Recognized by voice',
    manual: 'Confirmed by you',
    manual_unknown: 'Kept unidentified',
    unknown: 'Not identified',
  };

  let {
    speaker,
    name,
    people,
    onassign,
  }: {
    speaker: Speaker;
    name: string;
    people: Person[];
    onassign: (personId: string | null, newName?: string) => Promise<unknown>;
  } = $props();
  let adding = $state(false);
  let newName = $state('');
  let busy = $state(false);
  let error: unknown = $state(null);

  async function run(personId: string | null, created?: string) {
    busy = true;
    error = null;
    try {
      await onassign(personId, created);
      adding = false;
      newName = '';
    } catch (e) {
      error = e;
    } finally {
      busy = false;
    }
  }
  const suggestions = $derived(
    speaker.evidence.filter((x) => x.name && !x.applied && speaker.state !== 'manual'),
  );
  // A speaker is named automatically by their voice or by introducing themselves.
  const how = $derived(
    speaker.state === 'automatic' &&
      !speaker.evidence.some((x) => x.source === 'voice' && x.applied !== false)
      ? 'Introduced themselves'
      : STATES[speaker.state],
  );
</script>

<div class="speaker">
  <span class="speaker-name">{name}</span>
  <small>{how}</small>
  <ErrorNotice {error} />
  {#if adding}
    <form
      class="new-person"
      onsubmit={(e) => {
        e.preventDefault();
        if (newName.trim()) run(null, newName.trim());
      }}
    >
      <label class="sr-only" for={`new-${speaker.label}`}>New person’s name</label>
      <input
        id={`new-${speaker.label}`}
        {@attach (input) => input.focus()}
        maxlength={120}
        placeholder="Name"
        bind:value={newName}
      />
      <div>
        <button class="primary" disabled={busy || !newName.trim()}>Add</button>
        <button type="button" onclick={() => (adding = false)} disabled={busy}>Cancel</button>
      </div>
    </form>
  {:else}
    <label>
      <span class="sr-only">Who is {name}?</span>
      <select
        value={speaker.person_id ?? ''}
        disabled={busy}
        onchange={(e) =>
          e.currentTarget.value === NEW_PERSON
            ? (adding = true)
            : run(e.currentTarget.value || null)}
      >
        <option value="">Not identified</option>
        {#each people as p (p.id)}
          <option value={p.id}>{p.name}</option>
        {/each}
        <option value={NEW_PERSON}>New person…</option>
      </select>
    </label>
  {/if}
  {#if speaker.state === 'automatic' && speaker.person_id}
    <button class="text-button" disabled={busy} onclick={() => run(speaker.person_id)}>
      Confirm {name}
    </button>
  {/if}
  {#each suggestions as x (`${x.name}-${x.quote}`)}
    <div class="suggestion">
      <small>Introduced as {x.name}: “{x.quote}”</small>
      <button
        class="text-button"
        disabled={busy}
        onclick={() => {
          const known = people.find((p) => p.name.toLowerCase() === x.name!.toLowerCase());
          run(known?.id ?? null, known ? undefined : x.name);
        }}
      >
        This is {x.name}
      </button>
    </div>
  {/each}
</div>
