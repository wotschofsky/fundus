<script lang="ts">
  import { createMutation, useQueryClient } from '@tanstack/svelte-query';
  import { LogOut } from '@lucide/svelte';
  import { api, send, type Me } from '../api';
  import ConnectAppsSection from '../components/connect-apps-section.svelte';
  import ErrorNotice from '../components/error-notice.svelte';
  import ProcessingSection from '../components/processing-section.svelte';

  let { me }: { me: Me } = $props();
  const qc = useQueryClient();
  const signOut = createMutation(() => ({
    mutationFn: () => api('/v1/logout', send('POST')),
    onSuccess: () => {
      qc.clear();
      location.assign('/');
    },
  }));
</script>

<header class="page-header">
  <h1>Settings</h1>
  {#if me.auth_mode !== 'proxy'}
    <button disabled={signOut.isPending} onclick={() => signOut.mutate()}>
      <LogOut size={15} aria-hidden="true" />
      Sign out
    </button>
  {/if}
</header>
<ErrorNotice error={signOut.error} />
<ConnectAppsSection {me} />
<ProcessingSection />
