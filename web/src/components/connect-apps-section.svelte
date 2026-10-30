<!-- How the Mac app, chat apps (MCP) and token clients connect, and what is connected now. -->
<script lang="ts">
  import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { KeyRound } from '@lucide/svelte';
  import { api, send, type Connection, type Me, type Token } from '../api';
  import { date, dateTime } from '../format';
  import ConfirmDialog, { type ConfirmRequest } from './confirm-dialog.svelte';
  import CopyField from './copy-field.svelte';
  import ErrorNotice from './error-notice.svelte';

  let { me }: { me: Me } = $props();
  const qc = useQueryClient();
  let confirming: ConfirmRequest | null = $state(null);
  let name = $state('');
  let scope: 'upload' | 'read' = $state('upload');
  let secret = $state('');
  const connections = createQuery(() => ({
    queryKey: ['oauth-connections'],
    queryFn: () => api<{ connections: Connection[] }>('/v1/oauth/connections'),
  }));
  const tokens = createQuery(() => ({
    queryKey: ['tokens'],
    queryFn: () => api<{ tokens: Token[] }>('/v1/tokens'),
  }));
  const create = createMutation(() => ({
    mutationFn: () => api<{ token: string }>('/v1/tokens', send('POST', { name, scope })),
    onSuccess: (v) => {
      secret = v.token;
      name = '';
      return qc.invalidateQueries({ queryKey: ['tokens'] });
    },
  }));
  const mcpUrl = $derived(new URL('mcp', me.public_url).href);
</script>

<section class="settings-section" id="connect">
  <h2>Connect apps</h2>
  <div class="setting">
    <h3>Mac app</h3>
    <p>Enter this address in the app’s Settings, then approve access in your browser.</p>
    <CopyField value={me.public_url} label="Copy server address" />
  </div>
  <div class="setting">
    <h3>Chat apps</h3>
    <p>
      Add this MCP address as a connector in your chat app and choose Connect. You approve read-only
      access here; the app can search and read transcripts, never change them.
    </p>
    <CopyField value={mcpUrl} label="Copy MCP address" />
  </div>
  {#if connections.data?.connections.length}
    <div class="setting">
      <h3>Connected apps</h3>
      <div class="rows">
        {#each connections.data.connections as c (c.id)}
          <div class="connection">
            <div>
              <strong>{c.name}</strong>
              <small>
                {c.scope === 'vault:owner' ? 'Full access' : 'Read only'} · Connected
                {new Date(c.created_at * 1000).toLocaleDateString()}
              </small>
              <small>
                {c.last_used_at ? `Last used ${dateTime(c.last_used_at)}` : 'No usage recorded'}
              </small>
            </div>
            <button
              onclick={() =>
                (confirming = {
                  title: `Disconnect ${c.name}?`,
                  text: 'It loses access immediately. You can connect it again at any time.',
                  action: 'Disconnect',
                  danger: true,
                  run: () =>
                    api(`/v1/oauth/connections/${c.id}`, send('DELETE')).then(() =>
                      qc.invalidateQueries({ queryKey: ['oauth-connections'] }),
                    ),
                })}
            >
              Disconnect
            </button>
          </div>
        {/each}
      </div>
    </div>
  {/if}
  <div class="setting">
    <h3>Access tokens</h3>
    <p>
      For the mobile app and other clients without browser sign-in. Upload tokens can only add
      recordings; read tokens can only search and read.
    </p>
    <form
      class="token-form"
      onsubmit={(e) => {
        e.preventDefault();
        create.mutate();
      }}
    >
      <label>
        Name
        <input placeholder="iPhone" bind:value={name} required maxlength={100} />
      </label>
      <label>
        Access
        <select bind:value={scope}>
          <option value="upload">Upload recordings</option>
          <option value="read">Read recordings</option>
        </select>
      </label>
      <button class="primary" disabled={create.isPending}>
        <KeyRound size={15} aria-hidden="true" />
        Create token
      </button>
    </form>
    <ErrorNotice error={create.error ?? tokens.error ?? connections.error} />
    {#if secret}
      <div class="notice secret" role="status">
        <strong>Copy this token now. It is shown only once.</strong>
        <CopyField value={secret} label="Copy token" />
        <button onclick={() => (secret = '')}>Done</button>
      </div>
    {/if}
    {#if tokens.data?.tokens.length}
      <div class="rows">
        {#each tokens.data.tokens as t (t.id)}
          <div class="connection">
            <div>
              <strong>{t.name}</strong>
              <small>
                {t.scope === 'read' ? 'Read recordings' : 'Upload recordings'} · Created
                {date(t.created_at)}
              </small>
              <small>
                {t.last_used_at ? `Last used ${dateTime(t.last_used_at)}` : 'No usage recorded'}
              </small>
            </div>
            <button
              class="danger"
              onclick={() =>
                (confirming = {
                  title: `Revoke “${t.name}”?`,
                  text: 'Clients using this token lose access immediately.',
                  action: 'Revoke',
                  danger: true,
                  run: () =>
                    api(`/v1/tokens/${t.id}`, send('DELETE')).then(() =>
                      qc.invalidateQueries({ queryKey: ['tokens'] }),
                    ),
                })}
            >
              Revoke
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>
  <ConfirmDialog bind:request={confirming} />
</section>
