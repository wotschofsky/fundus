import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { KeyRound } from 'lucide-react';
import { api, send, type Connection, type Me, type Token } from '../api';
import { date } from '../format';
import { useConfirm } from '../hooks/use-confirm';
import { ConfirmDialog } from './confirm-dialog';
import { CopyField } from './copy-field';
import { ErrorNotice } from './error-notice';

/** How the Mac app, chat apps (MCP) and token clients connect, and what is connected now. */
export function ConnectAppsSection({ me }: { me: Me }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [name, setName] = useState('');
  const [scope, setScope] = useState<'upload' | 'read'>('upload');
  const [secret, setSecret] = useState('');
  const connections = useQuery({
    queryKey: ['oauth-connections'],
    queryFn: () => api<{ connections: Connection[] }>('/v1/oauth/connections'),
  });
  const tokens = useQuery({
    queryKey: ['tokens'],
    queryFn: () => api<{ tokens: Token[] }>('/v1/tokens'),
  });
  const create = useMutation({
    mutationFn: () => api<{ token: string }>('/v1/tokens', send('POST', { name, scope })),
    onSuccess: (v) => {
      setSecret(v.token);
      setName('');
      return qc.invalidateQueries({ queryKey: ['tokens'] });
    },
  });
  const mcpUrl = new URL('mcp', me.public_url).href;
  return (
    <section className="settings-section" id="connect">
      <h2>Connect apps</h2>
      <div className="setting">
        <h3>Mac app</h3>
        <p>Enter this address in the app’s Settings, then approve access in your browser.</p>
        <CopyField value={me.public_url} label="Copy server address" />
      </div>
      <div className="setting">
        <h3>Chat apps</h3>
        <p>
          Add this MCP address as a connector in your chat app and choose Connect. You approve
          read-only access here; the app can search and read transcripts, never change them.
        </p>
        <CopyField value={mcpUrl} label="Copy MCP address" />
      </div>
      {!!connections.data?.connections.length && (
        <div className="setting">
          <h3>Connected apps</h3>
          <div className="rows">
            {connections.data.connections.map((c) => (
              <div className="connection" key={c.id}>
                <div>
                  <strong>{c.name}</strong>
                  <small>
                    {c.scope === 'vault:owner' ? 'Full access' : 'Read only'} · Connected{' '}
                    {new Date(c.created_at * 1000).toLocaleDateString()}
                  </small>
                </div>
                <button
                  onClick={() =>
                    confirm.ask({
                      title: `Disconnect ${c.name}?`,
                      text: 'It loses access immediately. You can connect it again at any time.',
                      action: 'Disconnect',
                      danger: true,
                      run: () =>
                        api(`/v1/oauth/connections/${c.id}`, send('DELETE')).then(() =>
                          qc.invalidateQueries({ queryKey: ['oauth-connections'] }),
                        ),
                    })
                  }
                >
                  Disconnect
                </button>
              </div>
            ))}
          </div>
        </div>
      )}
      <div className="setting">
        <h3>Access tokens</h3>
        <p>
          For the mobile app and other clients without browser sign-in. Upload tokens can only add
          recordings; read tokens can only search and read.
        </p>
        <form
          className="token-form"
          onSubmit={(e) => {
            e.preventDefault();
            create.mutate();
          }}
        >
          <label>
            Name
            <input
              placeholder="iPhone"
              value={name}
              onChange={(e) => setName(e.target.value)}
              required
              maxLength={100}
            />
          </label>
          <label>
            Access
            <select value={scope} onChange={(e) => setScope(e.target.value as 'upload' | 'read')}>
              <option value="upload">Upload recordings</option>
              <option value="read">Read recordings</option>
            </select>
          </label>
          <button className="primary" disabled={create.isPending}>
            <KeyRound size={15} aria-hidden />
            Create token
          </button>
        </form>
        <ErrorNotice error={create.error ?? tokens.error ?? connections.error} />
        {secret && (
          <div className="notice secret" role="status">
            <strong>Copy this token now. It is shown only once.</strong>
            <CopyField value={secret} label="Copy token" />
            <button onClick={() => setSecret('')}>Done</button>
          </div>
        )}
        {!!tokens.data?.tokens.length && (
          <div className="rows">
            {tokens.data.tokens.map((t) => (
              <div className="connection" key={t.id}>
                <div>
                  <strong>{t.name}</strong>
                  <small>
                    {t.scope === 'read' ? 'Read recordings' : 'Upload recordings'} · Created{' '}
                    {date(t.created_at)}
                  </small>
                </div>
                <button
                  className="danger"
                  onClick={() =>
                    confirm.ask({
                      title: `Revoke “${t.name}”?`,
                      text: 'Clients using this token lose access immediately.',
                      action: 'Revoke',
                      danger: true,
                      run: () =>
                        api(`/v1/tokens/${t.id}`, send('DELETE')).then(() =>
                          qc.invalidateQueries({ queryKey: ['tokens'] }),
                        ),
                    })
                  }
                >
                  Revoke
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
      <ConfirmDialog {...confirm.dialog} />
    </section>
  );
}
