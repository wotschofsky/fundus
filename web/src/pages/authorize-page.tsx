import { useMutation, useQuery } from '@tanstack/react-query';
import { api, send, type Authorization } from '../api';
import { ErrorNotice } from '../components/error-notice';
import { LoadingState } from '../components/loading-state';

export function AuthorizePage() {
  const search = window.location.search;
  const request = useQuery({
    queryKey: ['oauth-authorization', search],
    queryFn: () => api<Authorization>(`/v1/oauth/authorize${search}`),
    retry: false,
    refetchOnWindowFocus: false,
  });
  const consent = useMutation({
    mutationFn: (approve: boolean) =>
      api<{ redirect_url: string }>(
        '/v1/oauth/authorize',
        send('POST', { request_id: request.data!.request_id, approve }),
      ),
    onSuccess: (value) => window.location.assign(value.redirect_url),
  });
  if (request.isPending) return <LoadingState />;
  const data = request.data;
  const owner = data?.scope === 'vault:owner';
  return (
    <main className="login oauth-consent">
      <h1>Connect to your vault</h1>
      {data && (
        <>
          <p>
            <strong>{data.client_name}</strong> is requesting{' '}
            {owner ? 'full access to your vault' : 'read access to your recordings'}.
          </p>
          <p>
            {owner
              ? 'It will be able to upload recordings, play and read your archive, edit transcripts, speakers, people and labels, and delete recordings.'
              : 'It will be able to search recordings and read transcripts, summaries, labels and speaker names. It cannot upload, edit or delete anything.'}
          </p>
          <div className="oauth-destination">
            <span>Returns to</span>
            <code>{data.redirect_uri}</code>
          </div>
          <p className="oauth-hint">
            Approve only if you just started connecting this app. Its name is supplied by the app
            itself.
          </p>
        </>
      )}
      <ErrorNotice error={request.error ?? consent.error} />
      {data && (
        <div className="oauth-actions">
          <button
            disabled={consent.isPending || consent.isSuccess}
            onClick={() => consent.mutate(false)}
          >
            Cancel
          </button>
          <button
            className="primary"
            disabled={consent.isPending || consent.isSuccess}
            onClick={() => consent.mutate(true)}
          >
            {consent.isPending ? 'Connecting…' : owner ? 'Allow full access' : 'Allow read access'}
          </button>
        </div>
      )}
      <small>You can disconnect this app at any time in Settings.</small>
    </main>
  );
}
