<script lang="ts">
  import { createMutation, useQueryClient } from '@tanstack/svelte-query';
  import { ArrowRight } from '@lucide/svelte';
  import { api, send } from '../api';
  import ErrorNotice from '../components/error-notice.svelte';

  const qc = useQueryClient();
  let password = $state('');
  const login = createMutation(() => ({
    mutationFn: () => api('/login', send('POST', { password })),
    onSuccess: () => qc.invalidateQueries(),
  }));
</script>

<main class="login">
  <h1>Vault</h1>
  <p>Sign in to your recording archive.</p>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      login.mutate();
    }}
  >
    <label for="password">Password</label>
    <input
      id="password"
      type="password"
      autocomplete="current-password"
      required
      {@attach (input) => input.focus()}
      bind:value={password}
    />
    <ErrorNotice error={login.error} />
    <button class="primary" disabled={login.isPending}>
      {login.isPending ? 'Signing in…' : 'Sign in'}
      <ArrowRight size={16} aria-hidden="true" />
    </button>
  </form>
  <small>Your archive lives on your own server.</small>
</main>
