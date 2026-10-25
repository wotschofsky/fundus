import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { RotateCcw } from 'lucide-react';
import { api, send } from '../api';
import { date, duration } from '../format';
import { useConfirm } from '../hooks/use-confirm';
import { useEntries } from '../hooks/use-entries';
import { BackLink } from '../components/back-link';
import { ConfirmDialog } from '../components/confirm-dialog';
import { EmptyState } from '../components/empty-state';
import { ErrorNotice } from '../components/error-notice';
import { LoadMoreButton } from '../components/load-more-button';
import { LoadingState } from '../components/loading-state';

export function TrashPage() {
  const qc = useQueryClient();
  const entries = useEntries('&trash=true', false);
  const confirm = useConfirm();
  const [error, setError] = useState<unknown>(null);
  const all = entries.data ?? [];
  // Labels count only recordings outside Trash, and search shows restored ones again.
  const refresh = () =>
    Promise.all(
      ['entries', 'labels', 'search'].map((key) => qc.invalidateQueries({ queryKey: [key] })),
    );
  return (
    <>
      <BackLink />
      <header className="page-header">
        <h1>Trash</h1>
      </header>
      <p className="helper">
        Recordings in Trash are hidden from the library, search and chat apps. Restore them at any
        time; deleting permanently removes their audio and transcript from your server.
      </p>
      <ErrorNotice error={entries.error ?? error} />
      {entries.isPending ? (
        <LoadingState />
      ) : all.length === 0 ? (
        <EmptyState title="Trash is empty" text="Recordings you move to Trash appear here." />
      ) : (
        <div className="rows">
          {all.map((e) => (
            <div className="trash-row" key={e.id}>
              <div>
                <div className="recording-meta">
                  <span>{date(e.recorded_at)}</span>
                  <span>{duration(e.duration_ms)}</span>
                </div>
                <h3>{e.title}</h3>
              </div>
              <div className="row-actions">
                <button
                  onClick={() =>
                    api(`/v1/entries/${e.id}/restore`, send('POST')).then(() => {
                      setError(null);
                      return refresh();
                    }, setError)
                  }
                >
                  <RotateCcw size={14} aria-hidden />
                  Restore
                </button>
                <button
                  className="danger"
                  onClick={() =>
                    confirm.ask({
                      title: `Delete “${e.title}” permanently?`,
                      text: 'Its archived audio, transcript and contributed voice samples are removed from your server. This cannot be undone. Originals on your recorder are not affected.',
                      action: 'Delete permanently',
                      danger: true,
                      run: () => api(`/v1/entries/${e.id}`, send('DELETE')).then(refresh),
                    })
                  }
                >
                  Delete permanently
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
      <LoadMoreButton query={entries} />
      <ConfirmDialog {...confirm.dialog} />
    </>
  );
}
