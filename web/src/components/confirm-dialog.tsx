import { useEffect, useRef, useState } from 'react';
import type { ConfirmRequest } from '../hooks/use-confirm';
import { ErrorNotice } from './error-notice';

/** Native modal <dialog>; the request's action runs only after confirmation. */
export function ConfirmDialog({
  request,
  onClose,
}: {
  request: ConfirmRequest | null;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (request && dialog && !dialog.open) dialog.showModal();
    if (!request && dialog?.open) dialog.close();
    setError(null);
  }, [request]);
  const close = () => {
    if (!pending) onClose();
  };
  return (
    <dialog
      ref={ref}
      className="confirm"
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
    >
      {request && (
        <form
          method="dialog"
          onSubmit={async (event) => {
            event.preventDefault();
            setPending(true);
            setError(null);
            try {
              await request.run();
              onClose();
            } catch (e) {
              setError(e);
            } finally {
              setPending(false);
            }
          }}
        >
          <h2>{request.title}</h2>
          <p>{request.text}</p>
          <ErrorNotice error={error} />
          <div className="dialog-actions">
            <button type="button" onClick={close} disabled={pending}>
              Cancel
            </button>
            <button className={request.danger ? 'danger' : 'primary'} disabled={pending}>
              {pending ? 'Working…' : request.action}
            </button>
          </div>
        </form>
      )}
    </dialog>
  );
}
