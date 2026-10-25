import { useState } from 'react';
import type { Person, Speaker } from '../api';
import { ErrorNotice } from './error-notice';

/** Select value for the "New person…" option; person ids are UUIDs, so it cannot collide. */
const NEW_PERSON = '+new';

const STATES: Record<Speaker['state'], string> = {
  automatic: 'Recognized by voice',
  manual: 'Confirmed by you',
  manual_unknown: 'Kept unidentified',
  unknown: 'Not identified',
};

/** Who a diarized speaker is: pick a person, add one, confirm a voice match or a self-introduction. */
export function SpeakerControl({
  speaker,
  name,
  people,
  onAssign,
}: {
  speaker: Speaker;
  name: string;
  people: Person[];
  onAssign: (personId: string | null, newName?: string) => Promise<unknown>;
}) {
  const [adding, setAdding] = useState(false);
  const [newName, setNewName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const run = async (personId: string | null, created?: string) => {
    setBusy(true);
    setError(null);
    try {
      await onAssign(personId, created);
      setAdding(false);
      setNewName('');
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  };
  const suggestions = speaker.evidence.filter(
    (x) => x.name && !x.applied && speaker.state !== 'manual',
  );
  return (
    <div className="speaker">
      <span className="speaker-name">{name}</span>
      <small>{STATES[speaker.state]}</small>
      <ErrorNotice error={error} />
      {adding ? (
        <form
          className="new-person"
          onSubmit={(e) => {
            e.preventDefault();
            if (newName.trim()) run(null, newName.trim());
          }}
        >
          <label className="sr-only" htmlFor={`new-${speaker.label}`}>
            New person’s name
          </label>
          <input
            id={`new-${speaker.label}`}
            autoFocus
            maxLength={120}
            placeholder="Name"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
          />
          <div>
            <button className="primary" disabled={busy || !newName.trim()}>
              Add
            </button>
            <button type="button" onClick={() => setAdding(false)} disabled={busy}>
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <label>
          <span className="sr-only">Who is {name}?</span>
          <select
            value={speaker.person_id ?? ''}
            disabled={busy}
            onChange={(e) =>
              e.target.value === NEW_PERSON ? setAdding(true) : run(e.target.value || null)
            }
          >
            <option value="">Not identified</option>
            {people.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
            <option value={NEW_PERSON}>New person…</option>
          </select>
        </label>
      )}
      {speaker.state === 'automatic' && speaker.person_id && (
        <button className="text-button" disabled={busy} onClick={() => run(speaker.person_id)}>
          Confirm {name}
        </button>
      )}
      {suggestions.map((x) => (
        <div className="suggestion" key={`${x.name}-${x.quote}`}>
          <small>
            Introduced as {x.name}: “{x.quote}”
          </small>
          <button
            className="text-button"
            disabled={busy}
            onClick={() => {
              const known = people.find((p) => p.name.toLowerCase() === x.name!.toLowerCase());
              run(known?.id ?? null, known ? undefined : x.name);
            }}
          >
            This is {x.name}
          </button>
        </div>
      ))}
    </div>
  );
}
