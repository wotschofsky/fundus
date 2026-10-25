import { Link, NavLink, Route, Routes } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { api, ApiError, type Me } from './api';
import { useNavigationShortcuts } from './hooks/use-navigation-shortcuts';
import { EmptyState } from './components/empty-state';
import { ErrorNotice } from './components/error-notice';
import { LoadingState } from './components/loading-state';
import { AuthorizePage } from './pages/authorize-page';
import { LoginPage } from './pages/login-page';
import { PeoplePage } from './pages/people-page';
import { RecordingPage } from './pages/recording-page';
import { RecordingsPage } from './pages/recordings-page';
import { SearchPage } from './pages/search-page';
import { SettingsPage } from './pages/settings-page';
import { TrashPage } from './pages/trash-page';

export function App() {
  useNavigationShortcuts();
  const me = useQuery({ queryKey: ['me'], queryFn: () => api<Me>('/v1/me'), retry: false });
  if (me.isPending) return <LoadingState label="Opening your vault…" />;
  if (me.error instanceof ApiError && me.error.status === 401) return <LoginPage />;
  if (!me.data)
    return (
      <main className="login">
        <ErrorNotice error={me.error} />
        <button onClick={() => me.refetch()}>Try again</button>
      </main>
    );
  if (window.location.pathname === '/oauth/authorize') return <AuthorizePage />;
  return (
    <div className="shell">
      <header className="app-header">
        <Link className="brand" to="/">
          Vault
        </Link>
        <nav aria-label="Main">
          <NavLink to="/" end>
            Recordings
          </NavLink>
          <NavLink to="/search">Search</NavLink>
          <NavLink to="/people">People</NavLink>
          <NavLink to="/settings">Settings</NavLink>
        </nav>
      </header>
      <main className="workspace">
        <Routes>
          <Route path="/" element={<RecordingsPage />} />
          <Route path="/trash" element={<TrashPage />} />
          <Route path="/entries/:id" element={<RecordingPage />} />
          <Route path="/search" element={<SearchPage />} />
          <Route path="/people" element={<PeoplePage />} />
          <Route path="/settings" element={<SettingsPage me={me.data} />} />
          <Route
            path="*"
            element={
              <EmptyState
                title="Page not found"
                text="Return to your recordings to find what you need."
              >
                <Link className="button" to="/">
                  Recordings
                </Link>
              </EmptyState>
            }
          />
        </Routes>
      </main>
    </div>
  );
}
