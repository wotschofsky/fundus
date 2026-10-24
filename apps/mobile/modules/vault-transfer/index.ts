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
  configure: (server: string, token: string, timezone: string): Promise<void> =>
    native.configure(server, token, timezone),
  enqueue: (files: { uri: string; name: string }[]): Promise<void> => native.enqueue(files),
  resume: (): Promise<void> => native.resume(),
  pause: (): Promise<void> => native.pause(),
  clear: (): Promise<void> => native.clear(),
};
