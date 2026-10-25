import { useState } from 'react';
import type { Entry } from '../api';
import { localInput } from '../format';

export type EntryEdit = { title: string; labels: string[]; recorded_at?: string };

/** Title, recording time (in the viewer's local time) and labels of one recording. */
export function EditDetailsForm({
  entry,
  onSave,
  onCancel,
}: {
  entry: Entry;
  onSave: (edit: EntryEdit) => Promise<unknown>;
  onCancel: () => void;
}) {
  const initialTime = localInput(entry.recorded_at);
  const [title, setTitle] = useState(entry.title);
  const [labels, setLabels] = useState(entry.labels.join(', '));
  const [recordedAt, setRecordedAt] = useState(initialTime);
  const [saving, setSaving] = useState(false);
  return (
    <form
      className="edit-form"
      onSubmit={async (event) => {
        event.preventDefault();
        setSaving(true);
        await onSave({
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
        setSaving(false);
      }}
    >
      <label>
        Title
        <input value={title} maxLength={240} required onChange={(e) => setTitle(e.target.value)} />
      </label>
      <label>
        Recorded at <span className="hint">your local time</span>
        <input
          type="datetime-local"
          value={recordedAt}
          onChange={(e) => setRecordedAt(e.target.value)}
        />
      </label>
      <label>
        Labels <span className="hint">separated by commas</span>
        <input
          value={labels}
          placeholder="work, ideas"
          onChange={(e) => setLabels(e.target.value)}
        />
      </label>
      <div>
        <button className="primary" disabled={saving}>
          Save
        </button>
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
