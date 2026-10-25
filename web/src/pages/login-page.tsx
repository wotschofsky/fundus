import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ArrowRight } from 'lucide-react';
import { api, send } from '../api';
import { ErrorNotice } from '../components/error-notice';

export function LoginPage() {
  const [password, setPassword] = useState('');
  const qc = useQueryClient();
  const login = useMutation({
    mutationFn: () => api('/login', send('POST', { password })),
    onSuccess: () => qc.invalidateQueries(),
  });
  return (
    <main className="login">
      <h1>Vault</h1>
      <p>Sign in to your recording archive.</p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          login.mutate();
        }}
      >
        <label htmlFor="password">Password</label>
        <input
          id="password"
          type="password"
          autoComplete="current-password"
          required
          autoFocus
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <ErrorNotice error={login.error} />
        <button className="primary" disabled={login.isPending}>
          {login.isPending ? 'Signing in…' : 'Sign in'}
          <ArrowRight size={16} aria-hidden />
        </button>
      </form>
      <small>Your archive lives on your own server.</small>
    </main>
  );
}
