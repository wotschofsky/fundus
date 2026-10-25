import { useState } from 'react';

export type ConfirmRequest = {
  title: string;
  text: string;
  action: string;
  danger?: boolean;
  run: () => Promise<unknown>;
};

/** `ask(...)` opens the page's <ConfirmDialog {...dialog} />. */
export function useConfirm() {
  const [request, setRequest] = useState<ConfirmRequest | null>(null);
  return { ask: setRequest, dialog: { request, onClose: () => setRequest(null) } };
}
