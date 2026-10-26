<script module lang="ts">
  export type EntryEdit = { title: string; labels: string[]; recorded_at?: string };
</script>

<!-- Title, recording time (in the viewer's local time) and labels of one recording. -->
<script lang="ts">
  import type { Entry } from '../api';
  import { localInput } from '../format';

  let {
    entry,
    onsave,
    oncancel,
  }: { entry: Entry; onsave: (edit: EntryEdit) => Promise<unknown>; oncancel: () => void } =
    $props();
  // The form edits a copy; later refetches of the entry do not overwrite what is being typed.
  // svelte-ignore state_referenced_locally
  const initial = { ...entry, time: localInput(entry.recorded_at) };
  const initialTime = initial.time;
  let title = $state(initial.title);
  let labels = $state(initial.labels.join(', '));
  let recordedAt = $state(initialTime);
  let saving = $state(false);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    saving = true;
    await onsave({
      title,
      labels: labels
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean),
      // Only send the time when it changed: the field has minute precision.
      ...(recordedAt && recordedAt !== initialTime
        ? { recorded_at: new Date(recordedAt).toISOString() }
        : {}),
    });
    saving = false;
  }
</script>

<form class="edit-form" onsubmit={submit}>
  <label>
    Title
    <input bind:value={title} maxlength={240} required />
  </label>
  <label>
    Recorded at <span class="hint">your local time</span>
    <input type="datetime-local" bind:value={recordedAt} />
  </label>
  <label>
    Labels <span class="hint">separated by commas</span>
    <input bind:value={labels} placeholder="work, ideas" />
  </label>
  <div>
    <button class="primary" disabled={saving}>Save</button>
    <button type="button" onclick={oncancel}>Cancel</button>
  </div>
</form>
