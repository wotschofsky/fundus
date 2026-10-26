<!-- Recording context and voice thresholds (editable); configured models (read only). -->
<script lang="ts">
  import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { api, send, type ProcessingSettings } from '../api';
  import ErrorNotice from './error-notice.svelte';

  /** Server defaults for voice matching. */
  const DEFAULT_THRESHOLDS = { strong: 0.78, margin: 0.1 };
  const MODEL_LABELS: Record<string, string> = {
    transcription: 'Transcription',
    text: 'Titles, summaries and names',
    embedding: 'Search by meaning',
    voice: 'Voice recognition',
  };
  /** Pipeline order; unknown keys last. */
  const order = (key: string) => {
    const index = Object.keys(MODEL_LABELS).indexOf(key);
    return index < 0 ? 99 : index;
  };
  const SLIDERS = [
    ['strong', 'Match threshold', 0.5, 0.99],
    ['margin', 'Lead over the next person', 0, 0.4],
  ] as const;

  const qc = useQueryClient();
  const settings = createQuery(() => ({
    queryKey: ['settings'],
    queryFn: () => api<ProcessingSettings>('/v1/settings'),
  }));
  let context = $state('');
  let thresholds = $state({ ...DEFAULT_THRESHOLDS });
  $effect(() => {
    const data = settings.data;
    if (!data) return;
    context = data.recording_context;
    thresholds = { ...data.voice_thresholds };
  });
  const save = createMutation(() => ({
    mutationFn: () =>
      api<ProcessingSettings>(
        '/v1/settings',
        send('PATCH', { recording_context: context, voice_thresholds: thresholds }),
      ),
    onSuccess: (v) => qc.setQueryData(['settings'], v),
  }));
  const data = $derived(settings.data);
  const dirty = $derived(
    !!data &&
      (context !== data.recording_context ||
        thresholds.strong !== data.voice_thresholds.strong ||
        thresholds.margin !== data.voice_thresholds.margin),
  );
</script>

<section class="settings-section">
  <h2>Processing</h2>
  <ErrorNotice error={settings.error ?? save.error} />
  {#if data}
    <form
      onsubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
    >
      <div class="setting">
        <label for="context"><h3>Recording context</h3></label>
        <p>
          Describe what your recordings usually contain: people, places, projects, languages. It
          helps with names, titles and summaries. It never rewrites what was said.
        </p>
        <textarea
          id="context"
          rows={5}
          maxlength={8000}
          bind:value={context}
          placeholder="Weekly product meetings with Morgan and Sam, mostly in German."></textarea>
        <small class="counter">{context.length.toLocaleString()} / 8,000</small>
      </div>
      <div class="setting">
        <h3>Voice matching</h3>
        <p>
          Higher values recognize fewer people automatically, but more safely. Changes apply to
          recordings processed from now on.
        </p>
        {#each SLIDERS as [key, label, min, max] (key)}
          <label class="slider">
            <span>{label}</span>
            <input type="range" {min} {max} step={0.01} bind:value={thresholds[key]} />
            <output>{thresholds[key].toFixed(2)}</output>
          </label>
        {/each}
      </div>
      <div class="form-actions">
        <button class="primary" disabled={!dirty || save.isPending}>
          {save.isPending ? 'Saving…' : 'Save'}
        </button>
        <button
          type="button"
          disabled={thresholds.strong === DEFAULT_THRESHOLDS.strong &&
            thresholds.margin === DEFAULT_THRESHOLDS.margin}
          onclick={() => (thresholds = { ...DEFAULT_THRESHOLDS })}
        >
          Reset voice matching
        </button>
        {#if save.isSuccess && !dirty}<span class="muted">Saved</span>{/if}
      </div>
    </form>
    <dl>
      {#each Object.entries(data.models).sort(([a], [b]) => order(a) - order(b)) as [key, value] (key)}
        <div>
          <dt>{MODEL_LABELS[key] ?? key}</dt>
          <dd>
            {key === 'voice' && !data.voice_configured ? 'Not configured on the server' : value}
          </dd>
        </div>
      {/each}
      <div>
        <dt>Default recorder time zone</dt>
        <dd>{data.recorder_timezone.replaceAll('_', ' ')}</dd>
      </div>
    </dl>
  {/if}
</section>
