import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Plus } from 'lucide-react';
import { api, peopleQuery, send, type Person } from '../api';
import { useConfirm } from '../hooks/use-confirm';
import { ConfirmDialog } from '../components/confirm-dialog';
import { EmptyState } from '../components/empty-state';
import { ErrorNotice } from '../components/error-notice';
import { LoadingState } from '../components/loading-state';
import { PersonRow } from '../components/person-row';

export function PeoplePage() {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [name, setName] = useState('');
  const people = useQuery(peopleQuery);
  const add = useMutation({
    mutationFn: () => api<Person>('/v1/people', send('POST', { name: name.trim() })),
    onSuccess: () => {
      setName('');
      return qc.invalidateQueries({ queryKey: ['people'] });
    },
  });
  return (
    <>
      <header className="page-header">
        <h1>People</h1>
      </header>
      <p className="helper">
        Confirm who is speaking in a transcript to build a voice profile. Future recordings then
        recognize them automatically; uncertain matches stay unidentified.
      </p>
      <form
        className="inline-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim()) add.mutate();
        }}
      >
        <label className="sr-only" htmlFor="person-name">
          Person’s name
        </label>
        <input
          id="person-name"
          placeholder="Add someone by name"
          maxLength={120}
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <button className="primary" disabled={add.isPending || !name.trim()}>
          <Plus size={16} aria-hidden />
          Add person
        </button>
      </form>
      <ErrorNotice error={add.error ?? people.error} />
      {people.isPending ? (
        <LoadingState />
      ) : people.data?.people.length === 0 ? (
        <EmptyState
          title="No people yet"
          text="Add someone here, or name a speaker directly in a transcript."
        />
      ) : (
        <div className="rows">
          {people.data?.people.map((p) => (
            <PersonRow key={p.id} person={p} confirm={confirm.ask} />
          ))}
        </div>
      )}
      <ConfirmDialog {...confirm.dialog} />
    </>
  );
}
