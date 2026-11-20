<!-- How the native apps and chat apps (MCP) connect through OAuth, and what is connected now. -->
<script lang="ts">
  import { createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { api, send, type Connection, type Me } from '../api';
  import { dateTime } from '../format';
  import ConfirmDialog, { type ConfirmRequest } from './confirm-dialog.svelte';
  import CopyField from './copy-field.svelte';
  import ErrorNotice from './error-notice.svelte';

  let { me }: { me: Me } = $props();
  const qc = useQueryClient();
  let confirming: ConfirmRequest | null = $state(null);
  const connections = createQuery(() => ({
    queryKey: ['oauth-connections'],
    queryFn: () => api<{ connections: Connection[] }>('/v1/oauth/connections'),
  }));
  const mcpUrl = $derived(new URL('mcp', me.public_url).href);
</script>

<section class="settings-section" id="connect">
  <h2>Connect apps</h2>
  <div class="setting">
    <h3>Mac, iPhone and Android apps</h3>
    <p>Enter this address when you connect the app, then approve access in your browser.</p>
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
  <ErrorNotice error={connections.error} />
  <ConfirmDialog bind:request={confirming} />
</section>
