import { useMutation, useQueryClient } from '@tanstack/react-query';
import { LogOut } from 'lucide-react';
import { api, send, type Me } from '../api';
import { ConnectAppsSection } from '../components/connect-apps-section';
import { ErrorNotice } from '../components/error-notice';
import { ProcessingSection } from '../components/processing-section';

export function SettingsPage({ me }: { me: Me }) {
  const qc = useQueryClient();
  const signOut = useMutation({
    mutationFn: () => api('/v1/logout', send('POST')),
    onSuccess: () => {
      qc.clear();
      location.assign('/');
    },
  });
  return (
    <>
      <header className="page-header">
        <h1>Settings</h1>
        {me.auth_mode !== 'proxy' && (
          <button disabled={signOut.isPending} onClick={() => signOut.mutate()}>
            <LogOut size={15} aria-hidden />
            Sign out
          </button>
        )}
      </header>
      <ErrorNotice error={signOut.error} />
      <ConnectAppsSection me={me} />
      <ProcessingSection />
    </>
  );
}
