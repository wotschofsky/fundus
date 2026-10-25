import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { api, send, type Person } from '../api';
import type { ConfirmRequest } from '../hooks/use-confirm';
import { ErrorNotice } from './error-notice';

/** One person; destructive actions go through the page's shared confirm dialog. */
export function PersonRow({
  person,
  confirm,
}: {
  person: Person;
  confirm: (request: ConfirmRequest) => void;
}) {
  const qc = useQueryClient();
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(person.name);
  const refresh = () => qc.invalidateQueries({ queryKey: ['people'] });
  const rename = useMutation({
    mutationFn: () => api(`/v1/people/${person.id}`, send('PATCH', { name: name.trim() })),
    onSuccess: async () => {
      await refresh();
      setRenaming(false);
    },
  });
  return (
    <div className="person">
      {renaming ? (
        <form
          className="inline-form"
          onSubmit={(e) => {
            e.preventDefault();
            if (name.trim()) rename.mutate();
          }}
        >
          <label className="sr-only" htmlFor={`rename-${person.id}`}>
            Name
          </label>
          <input
            id={`rename-${person.id}`}
            autoFocus
            maxLength={120}
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === 'Escape' && setRenaming(false)}
          />
          <button className="primary" disabled={rename.isPending || !name.trim()}>
            Save
          </button>
          <button type="button" onClick={() => setRenaming(false)}>
            Cancel
          </button>
        </form>
      ) : (
        <>
          <div className="person-body">
            <h2>{person.name}</h2>
            <p>
              {person.samples === 0
                ? 'No voice profile yet'
                : `${person.samples} confirmed voice sample${person.samples !== 1 ? 's' : ''}`}
            </p>
          </div>
          <div className="row-actions">
            <button
              onClick={() => {
                setName(person.name);
                setRenaming(true);
              }}
            >
              Rename
            </button>
            {person.samples > 0 && (
              <button
                onClick={() =>
                  confirm({
                    title: `Remove ${person.name}’s voice profile?`,
                    text: 'Their name and confirmed transcript assignments stay. New recordings will not recognize them until you confirm their voice again.',
                    action: 'Remove voice profile',
                    danger: true,
                    run: () => api(`/v1/people/${person.id}/voice`, send('DELETE')).then(refresh),
                  })
                }
              >
                Remove voice
              </button>
            )}
            <button
              className="danger"
              onClick={() =>
                confirm({
                  title: `Delete ${person.name}?`,
                  text: 'Their voice profile is deleted and their speakers become unidentified. Transcript text is kept.',
                  action: 'Delete person',
                  danger: true,
                  run: () =>
                    api(`/v1/people/${person.id}`, send('DELETE')).then(() =>
                      Promise.all([refresh(), qc.invalidateQueries({ queryKey: ['entry'] })]),
                    ),
                })
              }
            >
              Delete
            </button>
          </div>
        </>
      )}
      <ErrorNotice error={rename.error} />
    </div>
  );
}
