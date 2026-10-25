import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api, send, type ProcessingSettings } from '../api';
import { ErrorNotice } from './error-notice';

/** Server defaults for voice matching. */
const DEFAULT_THRESHOLDS = { strong: 0.82, margin: 0.1 };
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

/** Recording context and voice thresholds (editable); configured models (read only). */
export function ProcessingSection() {
  const qc = useQueryClient();
  const settings = useQuery({
    queryKey: ['settings'],
    queryFn: () => api<ProcessingSettings>('/v1/settings'),
  });
  const [context, setContext] = useState('');
  const [thresholds, setThresholds] = useState(DEFAULT_THRESHOLDS);
  useEffect(() => {
    if (!settings.data) return;
    setContext(settings.data.recording_context);
    setThresholds(settings.data.voice_thresholds);
  }, [settings.data]);
  const save = useMutation({
    mutationFn: () =>
      api<ProcessingSettings>(
        '/v1/settings',
        send('PATCH', { recording_context: context, voice_thresholds: thresholds }),
      ),
    onSuccess: (v) => qc.setQueryData(['settings'], v),
  });
  const data = settings.data;
  const dirty =
    !!data &&
    (context !== data.recording_context ||
      thresholds.strong !== data.voice_thresholds.strong ||
      thresholds.margin !== data.voice_thresholds.margin);
  return (
    <section className="settings-section">
      <h2>Processing</h2>
      <ErrorNotice error={settings.error ?? save.error} />
      {data && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate();
          }}
        >
          <div className="setting">
            <label htmlFor="context">
              <h3>Recording context</h3>
            </label>
            <p>
              Describe what your recordings usually contain: people, places, projects, languages. It
              helps with names, titles and summaries. It never rewrites what was said.
            </p>
            <textarea
              id="context"
              rows={5}
              maxLength={8000}
              value={context}
              placeholder="Weekly product meetings with Morgan and Sam, mostly in German."
              onChange={(e) => setContext(e.target.value)}
            />
            <small className="counter">{context.length.toLocaleString()} / 8,000</small>
          </div>
          <div className="setting">
            <h3>Voice matching</h3>
            <p>
              Higher values recognize fewer people automatically, but more safely. Changes apply to
              recordings processed from now on.
            </p>
            {(
              [
                ['strong', 'Match threshold', 0.5, 0.99],
                ['margin', 'Lead over the next person', 0, 0.4],
              ] as const
            ).map(([key, label, min, max]) => (
              <label className="slider" key={key}>
                <span>{label}</span>
                <input
                  type="range"
                  min={min}
                  max={max}
                  step={0.01}
                  value={thresholds[key]}
                  onChange={(e) => setThresholds({ ...thresholds, [key]: Number(e.target.value) })}
                />
                <output>{thresholds[key].toFixed(2)}</output>
              </label>
            ))}
          </div>
          <div className="form-actions">
            <button className="primary" disabled={!dirty || save.isPending}>
              {save.isPending ? 'Saving…' : 'Save'}
            </button>
            <button
              type="button"
              disabled={
                thresholds.strong === DEFAULT_THRESHOLDS.strong &&
                thresholds.margin === DEFAULT_THRESHOLDS.margin
              }
              onClick={() => setThresholds(DEFAULT_THRESHOLDS)}
            >
              Reset voice matching
            </button>
            {save.isSuccess && !dirty && <span className="muted">Saved</span>}
          </div>
        </form>
      )}
      {data && (
        <dl>
          {Object.entries(data.models)
            .sort(([a], [b]) => order(a) - order(b))
            .map(([key, value]) => (
              <div key={key}>
                <dt>{MODEL_LABELS[key] ?? key}</dt>
                <dd>
                  {key === 'voice' && !data.voice_configured
                    ? 'Not configured on the server'
                    : value}
                </dd>
              </div>
            ))}
          <div>
            <dt>Default recorder time zone</dt>
            <dd>{data.recorder_timezone.replaceAll('_', ' ')}</dd>
          </div>
        </dl>
      )}
    </section>
  );
}
