import { requireNativeModule } from 'expo-modules-core';
export type UploadFile = {
  id: string;
  name: string;
  state: string;
  length: number;
  offset: number;
};
export type QueueState = {
  server: string;
  timezone: string;
  state: string;
  error?: string;
  files: UploadFile[];
};
const native = requireNativeModule('VaultTransfer');
export const transfer = {
  status: async (): Promise<QueueState> => JSON.parse(await native.status()),
  /** Opens the vault's sign-in page in the browser; the connection never reaches JS. */
  signIn: (server: string, timezone: string): Promise<void> => native.signIn(server, timezone),
  /** Revokes this iPhone's access; refused while an import is unfinished. */
  signOut: (): Promise<void> => native.signOut(),
  enqueue: (files: { uri: string; name: string }[]): Promise<void> => native.enqueue(files),
  resume: (): Promise<void> => native.resume(),
  pause: (): Promise<void> => native.pause(),
  clear: (): Promise<void> => native.clear(),
};
