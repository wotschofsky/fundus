import React, { useState, useRef, useEffect } from 'react';
import { createRoot } from 'react-dom/client';
import {
  BrowserRouter,
  Routes,
  Route,
  Link,
  NavLink,
  useNavigate,
  useSearchParams,
  useParams,
} from 'react-router-dom';
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
  useMutation,
  useQueryClient,
  useInfiniteQuery,
} from '@tanstack/react-query';
import {
  ArrowLeft,
  ArrowRight,
  Search as SearchIcon,
  Play,
  LoaderCircle,
  Plus,
  KeyRound,
  LogOut,
  Copy,
  Check,
  RefreshCw,
  Trash2,
  FileAudio,
  ChevronDown,
} from 'lucide-react';
import { api, ApiError, Entry, Segment, duration, timestamp, date } from './api';
import './style.css';
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: (count, error) => !(error instanceof ApiError && error.status < 500) && count < 2,
      refetchOnWindowFocus: true,
    },
  },
});
function ErrorNotice({ error }: { error: unknown }) {
  return error ? (
    <div className="notice error" role="alert">
      {error instanceof Error ? error.message : 'Something went wrong. Please try again.'}
    </div>
  ) : null;
}
function Busy() {
  return (
    <div className="loading" role="status">
      <LoaderCircle className="spin" size={20} /> Loading your vault…
    </div>
  );
}
function Login() {
  const [password, setPassword] = useState('');
  const qc = useQueryClient();
  const login = useMutation({
    mutationFn: () => api('/login', { method: 'POST', body: JSON.stringify({ password }) }),
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
        <label htmlFor="password">Vault password</label>
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
          {login.isPending ? 'Signing in…' : 'Open vault'}
          <ArrowRight size={16} />
        </button>
      </form>
      <small>Your archive lives on your own server.</small>
    </main>
  );
}
function OAuthAuthorize() {
  const search = window.location.search;
  const request = useQuery({
    queryKey: ['oauth-authorization', search],
    queryFn: () => api(`/v1/oauth/authorize${search}`),
    retry: false,
    refetchOnWindowFocus: false,
  });
  const consent = useMutation({
    mutationFn: (approve: boolean) =>
      api('/v1/oauth/authorize', {
        method: 'POST',
        body: JSON.stringify({ request_id: request.data.request_id, approve }),
      }),
    onSuccess: (value) => window.location.assign(value.redirect_url),
  });
  if (request.isPending) return <Busy />;
  return (
    <main className="login oauth-consent">
      <h1>Connect to your vault</h1>
      {request.data && (
        <>
          <p>
            <strong>{request.data.client_name}</strong> is requesting{' '}
            {request.data.scope === 'vault:owner'
              ? 'owner access to your vault'
              : 'read access to your recording archive'}
            .
          </p>
          <p>
            {request.data.scope === 'vault:owner'
              ? 'This app will be able to upload recordings, read and play your archive, edit transcripts and speaker identities, manage people and labels, and delete recordings.'
              : 'This app will be able to search recordings and read transcripts, summaries, labels, and speaker information. It cannot upload, edit, or delete recordings.'}
          </p>
          <div className="oauth-destination">
            <span>Return address</span>
            <code>{request.data.redirect_uri}</code>
          </div>
          <p className="oauth-hint">
            Approve only if you started connecting this app. The app name is supplied by its
            developer.
          </p>
        </>
      )}
      <ErrorNotice error={request.error ?? consent.error} />
      {request.data && (
        <div className="oauth-actions">
          <button disabled={consent.isPending} onClick={() => consent.mutate(false)}>
            Cancel
          </button>
          <button
            className="primary"
            disabled={consent.isPending}
            onClick={() => consent.mutate(true)}
          >
            {consent.isPending
              ? 'Connecting…'
              : request.data.scope === 'vault:owner'
                ? 'Allow owner access'
                : 'Allow read access'}
          </button>
        </div>
      )}
      <small>You can disconnect this app at any time in your vault’s Settings.</small>
    </main>
  );
}
function OAuthConnections({ publicUrl }: { publicUrl: string }) {
  const qc = useQueryClient();
  const connections = useQuery({
    queryKey: ['oauth-connections'],
    queryFn: () => api('/v1/oauth/connections'),
  });
  const disconnect = useMutation({
    mutationFn: (id: string) => api(`/v1/oauth/connections/${id}`, { method: 'DELETE' }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['oauth-connections'] }),
  });
  const [copyError, setCopyError] = useState<unknown>(null);
  const mcpUrl = new URL('mcp', publicUrl).href;
  return (
    <section className="settings-section">
      <h2>Connect a chat app</h2>
      <p>
        Add this MCP server URL in your chat app, then choose Connect. Sign in and approve read
        access. No token copying is needed.
      </p>
      <div className="server-url">
        <span>{mcpUrl}</span>
        <button
          className="icon-button"
          aria-label="Copy MCP server address"
          onClick={() =>
            navigator.clipboard
              .writeText(mcpUrl)
              .catch(() => setCopyError(new Error('Select and copy the MCP server address.')))
          }
        >
          <Copy size={16} />
        </button>
      </div>
      <ErrorNotice error={connections.error ?? disconnect.error ?? copyError} />
      <div className="connections">
        {connections.data?.connections.map((connection: any) => (
          <div className="connection" key={connection.id}>
            <div>
              <strong>{connection.name}</strong>
              <small>
                Read recordings · Connected{' '}
                {new Date(connection.created_at * 1000).toLocaleDateString()}
              </small>
            </div>
            <button
              disabled={disconnect.isPending}
              onClick={() => disconnect.mutate(connection.id)}
            >
              Disconnect
            </button>
          </div>
        ))}
      </div>
    </section>
  );
}
function App() {
  const me = useQuery({ queryKey: ['me'], queryFn: () => api('/v1/me'), retry: false });
  if (me.isPending) return <Busy />;
  if (me.error instanceof ApiError && me.error.status === 401) return <Login />;
  if (me.error)
    return (
      <main className="login">
        <ErrorNotice error={me.error} />
        <button onClick={() => me.refetch()}>Retry connection</button>
      </main>
    );
  if (window.location.pathname === '/oauth/authorize') return <OAuthAuthorize />;
  return (
    <div className="shell">
      <header className="app-header">
        <Link className="brand" to="/">
          Vault
        </Link>
        <nav aria-label="Main navigation">
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
          <Route path="/" element={<Library />} />
          <Route path="/entries/:id" element={<Detail />} />
          <Route path="/search" element={<SearchPage />} />
          <Route path="/people" element={<People />} />
          <Route path="/settings" element={<Settings me={me.data} />} />
          <Route
            path="*"
            element={
              <Empty title="Page not found" text="Return to your library to find a recording." />
            }
          />
        </Routes>
      </main>
    </div>
  );
}
function Empty({ title, text }: { title: string; text: string }) {
  return (
    <section className="empty">
      <h2>{title}</h2>
      <p>{text}</p>
    </section>
  );
}
function Library() {
  const [label, setLabel] = useState('');
  const [stage, setStage] = useState('');
  const entries = useInfiniteQuery({
    queryKey: ['entries', label, stage],
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      api<{ entries: Entry[]; next_cursor: number | null }>(
        `/v1/entries?cursor=${pageParam}&labels=${encodeURIComponent(label)}&stage=${encodeURIComponent(stage)}`,
      ),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
    refetchInterval: 5000,
  });
  const labels = useQuery({ queryKey: ['labels'], queryFn: () => api('/v1/labels') });
  const all = entries.data?.pages.flatMap((p) => p.entries) ?? [];
  return (
    <>
      <header className="page-header">
        <div>
          <h1>Recordings</h1>
        </div>
        <Link className="button" to="/settings">
          Connect uploader
        </Link>
      </header>
      <div className="toolbar">
        <span>
          {all.length} recording{all.length !== 1 ? 's' : ''}
        </span>
        <div className="filters">
          <label>
            <span className="sr-only">Filter by label</span>
            <select value={label} onChange={(e) => setLabel(e.target.value)}>
              <option value="">All labels</option>
              {labels.data?.labels.map((l: any) => (
                <option key={l.name}>{l.name}</option>
              ))}
            </select>
          </label>
          <label>
            <span className="sr-only">Filter by status</span>
            <select value={stage} onChange={(e) => setStage(e.target.value)}>
              <option value="">All statuses</option>
              <option value="complete">Ready</option>
              <option value="failed">Needs attention</option>
              <option value="queued">Queued</option>
            </select>
          </label>
        </div>
      </div>
      <ErrorNotice error={entries.error} />
      {entries.isPending ? (
        <Busy />
      ) : all.length === 0 ? (
        <Empty
          title={label || stage ? 'No matching recordings' : 'No recordings yet'}
          text={
            label || stage
              ? 'Try another filter.'
              : 'Add audio with the upload app to start your archive.'
          }
        />
      ) : (
        <div className="recordings">
          {all.map((e) => (
            <Link className="recording" key={e.id} to={`/entries/${e.id}`}>
              <div className="recording-body">
                <div className="recording-meta">
                  <time>{date(e.recorded_at)}</time>
                  <span>{duration(e.duration_ms)}</span>
                  {e.stage !== 'complete' && (
                    <span className={`stage ${e.stage === 'failed' ? 'failed' : ''}`}>
                      {e.stage === 'failed' ? 'Needs attention' : e.stage}
                    </span>
                  )}
                </div>
                <h2>{e.title}</h2>
                <p>
                  {e.summary ??
                    (e.stage === 'failed' ? e.failure : 'Your recording is being prepared.')}
                </p>
                <div className="chips">
                  {e.labels.map((l) => (
                    <span key={l}>{l}</span>
                  ))}
                </div>
              </div>
            </Link>
          ))}
        </div>
      )}
      {entries.hasNextPage && (
        <button
          className="load-more"
          disabled={entries.isFetchingNextPage}
          onClick={() => entries.fetchNextPage()}
        >
          Load more recordings
          <ChevronDown size={16} />
        </button>
      )}
    </>
  );
}
function Detail() {
  const { id } = useParams();
  const [params] = useSearchParams();
  const audio = useRef<HTMLAudioElement>(null);
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState('');
  const [labelText, setLabelText] = useState('');
  const [recordedAt, setRecordedAt] = useState('');
  const [actionError, setActionError] = useState<unknown>(null);
  const [now, setNow] = useState(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const entry = useQuery({
    queryKey: ['entry', id],
    queryFn: () => api<Entry>(`/v1/entries/${id}`),
    refetchInterval: (q) =>
      q.state.data?.stage === 'complete' || q.state.data?.stage === 'failed' ? false : 3000,
  });
  const transcript = useInfiniteQuery({
    queryKey: ['transcript', id],
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      api<{ segments: Segment[]; next_cursor: number | null }>(
        `/v1/entries/${id}/transcript?cursor=${pageParam}`,
      ),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
    refetchInterval: entry.data?.stage === 'complete' ? false : 5000,
  });
  const people = useQuery({ queryKey: ['people'], queryFn: () => api('/v1/people') });
  const rows = transcript.data?.pages.flatMap((p) => p.segments) ?? [];
  const [follow, setFollow] = useState(false);
  useEffect(() => {
    if (follow && now > 0) {
      const row = rows.find((r) => now * 1000 >= r.start_ms && now * 1000 < r.end_ms);
      if (row)
        document.getElementById(row.id)?.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
  }, [Math.floor(now), follow]);
  function seek(ms: number) {
    if (audio.current) {
      audio.current.currentTime = ms / 1000;
      audio.current.play().catch(() => {});
    }
  }
  async function update(body: any) {
    setActionError(null);
    try {
      await api(`/v1/entries/${id}`, { method: 'PATCH', body: JSON.stringify(body) });
      await qc.invalidateQueries({ queryKey: ['entry', id] });
      qc.invalidateQueries({ queryKey: ['entries'] });
      qc.invalidateQueries({ queryKey: ['labels'] });
      setEditing(false);
    } catch (e) {
      setActionError(e);
    }
  }
  async function assign(label: string, person_id: string) {
    try {
      await api(`/v1/entries/${id}/speakers/${encodeURIComponent(label)}`, {
        method: 'PATCH',
        body: JSON.stringify({ person_id: person_id || null }),
      });
      qc.invalidateQueries({ queryKey: ['entry', id] });
      qc.invalidateQueries({ queryKey: ['transcript', id] });
      qc.invalidateQueries({ queryKey: ['people'] });
    } catch (e) {
      setActionError(e);
    }
  }
  if (entry.isPending) return <Busy />;
  if (!entry.data) return <ErrorNotice error={entry.error} />;
  const e = entry.data;
  return (
    <>
      <Link className="back" to="/">
        <ArrowLeft size={15} />
        Recordings
      </Link>
      <header className="detail-header">
        <div className="recording-meta">
          <time>{date(e.recorded_at)}</time>
          <span>{duration(e.duration_ms)}</span>
          <span>{e.timezone}</span>
        </div>
        <h1>{e.title}</h1>
        <div className="detail-actions">
          <div className="chips">
            {e.labels.map((l) => (
              <span key={l}>{l}</span>
            ))}
          </div>
          <button
            onClick={() => {
              setEditing(!editing);
              setTitle(e.title);
              setLabelText(e.labels.join(', '));
              setRecordedAt(
                e.recorded_at ? new Date(e.recorded_at).toISOString().slice(0, 16) : '',
              );
            }}
          >
            Edit details
          </button>
        </div>
      </header>
      {editing && (
        <form
          className="edit-form"
          onSubmit={(x) => {
            x.preventDefault();
            update({
              title,
              ...(recordedAt ? { recorded_at: new Date(recordedAt + 'Z').toISOString() } : {}),
              labels: labelText
                .split(',')
                .map((s) => s.trim())
                .filter(Boolean),
            });
          }}
        >
          <label>
            Title
            <input
              value={title}
              maxLength={240}
              onChange={(e) => setTitle(e.target.value)}
              required
            />
          </label>
          <label>
            Recorded at (UTC)
            <input
              type="datetime-local"
              value={recordedAt}
              onChange={(x) => setRecordedAt(x.target.value)}
            />
          </label>
          <label>
            Labels, separated by commas
            <input
              value={labelText}
              onChange={(e) => setLabelText(e.target.value)}
              placeholder="work, ideas"
            />
          </label>
          <div>
            <button className="primary">Save details</button>
            <button type="button" onClick={() => setEditing(false)}>
              Cancel
            </button>
          </div>
        </form>
      )}
      <ErrorNotice error={actionError} />
      {e.stage !== 'complete' && (
        <div className={`notice ${e.stage === 'failed' ? 'error' : ''}`}>
          <strong>{e.stage === 'failed' ? 'Processing stopped' : `Processing: ${e.stage}`}</strong>
          <p>{e.failure ?? 'You can leave this page. Your server will keep working.'}</p>
          {e.stage === 'failed' && (
            <button
              onClick={async () => {
                try {
                  await api(`/v1/entries/${id}/retry`, { method: 'POST' });
                  qc.invalidateQueries({ queryKey: ['entry', id] });
                } catch (x) {
                  setActionError(x);
                }
              }}
            >
              <RefreshCw size={14} />
              Retry unfinished work
            </button>
          )}
        </div>
      )}
      {e.warnings.length > 0 && (
        <details className="notice">
          <summary>
            {e.warnings.length} processing note{e.warnings.length !== 1 ? 's' : ''}
          </summary>
          {e.warnings.map((w) => (
            <p key={w}>{w}</p>
          ))}
        </details>
      )}
      {e.audio_available && (
        <div className="player">
          <audio
            ref={audio}
            controls
            preload="metadata"
            src={`/api/v1/entries/${id}/audio`}
            onTimeUpdate={(x) => setNow(x.currentTarget.currentTime)}
            onPlay={() => setIsPlaying(true)}
            onPause={() => setIsPlaying(false)}
            onLoadedMetadata={(x) => {
              const t = Number(params.get('t'));
              if (t > 0) x.currentTarget.currentTime = t;
            }}
          />
          <label>
            <input type="checkbox" checked={follow} onChange={(x) => setFollow(x.target.checked)} />
            Follow transcript
          </label>
        </div>
      )}
      <div className="detail-grid">
        <div>
          {e.summary && (
            <section className="summary">
              <div className="section-heading">
                <h2>Summary</h2>
                <span>Generated summary</span>
              </div>
              <p>{e.summary}</p>
            </section>
          )}
          <section className="transcript">
            <div className="section-heading">
              <h2>Transcript</h2>
              <span>Click a time to listen</span>
            </div>
            <ErrorNotice error={transcript.error} />
            {rows.length === 0 && (
              <p className="muted">The transcript will appear here when it is ready.</p>
            )}
            {rows.map((r) => (
              <article
                className={`turn ${isPlaying && now * 1000 >= r.start_ms && now * 1000 < r.end_ms ? 'speaking' : ''}`}
                id={r.id}
                key={r.id}
              >
                <button
                  className="time"
                  disabled={!e.audio_available}
                  onClick={() => seek(r.start_ms)}
                  aria-label={`Play from ${timestamp(r.start_ms)}`}
                >
                  <Play size={11} />
                  {timestamp(r.start_ms)}
                </button>
                <div>
                  <strong>{r.speaker_name ?? `Speaker ${r.speaker}`}</strong>
                  <p>{r.text}</p>
                </div>
              </article>
            ))}
            {transcript.hasNextPage && (
              <button
                className="load-more"
                onClick={() => transcript.fetchNextPage()}
                disabled={transcript.isFetchingNextPage}
              >
                Read more
                <ChevronDown size={15} />
              </button>
            )}
          </section>
        </div>
        <aside className="detail-side">
          <h2>Speakers</h2>
          {e.speakers?.map((s) => (
            <div className="speaker" key={s.label}>
              <span>{s.name ?? `Speaker ${s.label}`}</span>
              <small>
                {s.state === 'automatic'
                  ? 'Voice match'
                  : s.state === 'manual'
                    ? 'Confirmed'
                    : s.state === 'manual_unknown'
                      ? 'Kept unnamed'
                      : 'Not identified'}
              </small>
              <label>
                <span className="sr-only">Assign speaker {s.label}</span>
                <select value={s.person_id ?? ''} onChange={(x) => assign(s.label, x.target.value)}>
                  <option value="">Keep unidentified</option>
                  {people.data?.people.map((p: any) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
                    </option>
                  ))}
                </select>
              </label>
              {s.evidence
                ?.filter((x: any) => x.name && !x.applied)
                .map((x: any) => (
                  <small key={x.quote}>
                    Suggested: {x.name}
                    <br />“{x.quote}”
                  </small>
                ))}
            </div>
          ))}
          <Link className="text-link" to="/people">
            <Plus size={14} />
            Add a person
          </Link>
          {!!e.sources?.length && <h2 className="source-heading">Source recordings</h2>}
          {e.sources?.map((s, i) => (
            <div className="source" key={i}>
              <FileAudio size={14} />
              <span>{s.filename}</span>
            </div>
          ))}
          <button
            className="danger-text"
            onClick={async () => {
              if (
                !confirm(
                  'Delete this entry, its archived audio, transcript, and contributed voice samples? Recorder originals are not affected.',
                )
              )
                return;
              try {
                await api(`/v1/entries/${id}`, { method: 'DELETE' });
                qc.invalidateQueries({ queryKey: ['entries'] });
                navigate('/');
              } catch (x) {
                setActionError(x);
              }
            }}
          >
            <Trash2 size={14} />
            Delete recording
          </button>
        </aside>
      </div>
    </>
  );
}
function SearchPage() {
  const [params, setParams] = useSearchParams();
  const [query, setQuery] = useState(params.get('q') ?? '');
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [label, setLabel] = useState('');
  const q = params.get('q') ?? '';
  const result = useQuery({
    queryKey: ['search', q, from, to, label],
    queryFn: () =>
      api(
        `/v1/search?q=${encodeURIComponent(q)}${from ? `&from=${from}` : ''}${to ? `&to=${to}` : ''}${label ? `&labels=${encodeURIComponent(label)}` : ''}`,
      ),
    enabled: !!q,
  });
  return (
    <>
      <header className="page-header">
        <div>
          <h1>Search</h1>
        </div>
      </header>
      <form
        className="search-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (query.trim()) setParams({ q: query.trim() });
        }}
      >
        <SearchIcon size={22} />
        <input
          aria-label="Search recordings"
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search transcripts"
        />
        <button className="primary" aria-label="Search" disabled={!query.trim()}>
          Search
          <ArrowRight size={16} />
        </button>
      </form>
      <div className="search-filters">
        <label>
          From (UTC)
          <input type="date" value={from} onChange={(e) => setFrom(e.target.value)} />
        </label>
        <label>
          Through (UTC)
          <input type="date" value={to} min={from} onChange={(e) => setTo(e.target.value)} />
        </label>
        <label>
          Label
          <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Any label" />
        </label>
      </div>
      <ErrorNotice error={result.error} />
      {result.isFetching && <Busy />}
      {result.data && (
        <>
          <div className="toolbar">
            <span>
              {result.data.results.length} passage{result.data.results.length !== 1 ? 's' : ''}
            </span>
            <span>
              {result.data.mode === 'hybrid' ? 'Keyword + semantic search' : 'Keyword search'}
            </span>
          </div>
          {result.data.partial_results && (
            <div className="notice">
              Search coverage is incomplete. {result.data.coverage.pending_entries} recording(s) are
              unfinished; {result.data.coverage.passages_without_embeddings} passage(s) lack
              embeddings. Browse the library for full chronological coverage.
            </div>
          )}
          {result.data.results.length === 0 ? (
            <Empty
              title="No matching passages"
              text="Try a different phrase or browse recordings from the date you remember."
            />
          ) : (
            <div className="search-results">
              {result.data.results.map((r: any) => (
                <Link
                  key={r.id}
                  className="search-result"
                  to={`/entries/${r.entry_id}?t=${r.start_ms / 1000}`}
                >
                  <div className="recording-meta">
                    <span>{date(r.recorded_at)}</span>
                    <span>
                      {timestamp(r.start_ms)}–{timestamp(r.end_ms)}
                    </span>
                  </div>
                  <h2>{r.title}</h2>
                  <p>{r.excerpt}</p>
                  <span className="text-link">
                    Open passage
                    <ArrowRight size={14} />
                  </span>
                </Link>
              ))}
            </div>
          )}
        </>
      )}
      {!q && <Empty title="Find a recording" text="Enter a word, a phrase, or a topic." />}
    </>
  );
}
function People() {
  const qc = useQueryClient();
  const [actionError, setActionError] = useState<unknown>(null);
  const [name, setName] = useState('');
  const people = useQuery({ queryKey: ['people'], queryFn: () => api('/v1/people') });
  const add = useMutation({
    mutationFn: () => api('/v1/people', { method: 'POST', body: JSON.stringify({ name }) }),
    onSuccess: () => {
      setName('');
      qc.invalidateQueries({ queryKey: ['people'] });
    },
  });
  return (
    <>
      <header className="page-header">
        <div>
          <h1>People</h1>
        </div>
      </header>
      <form
        className="inline-form"
        onSubmit={(e) => {
          e.preventDefault();
          add.mutate();
        }}
      >
        <label className="sr-only" htmlFor="person-name">
          Person’s name
        </label>
        <input
          id="person-name"
          placeholder="Add someone’s name"
          required
          maxLength={120}
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <button className="primary" disabled={add.isPending}>
          <Plus size={16} />
          Add person
        </button>
      </form>
      <ErrorNotice error={actionError ?? add.error ?? people.error} />
      <p className="helper">
        Confirm a speaker in a transcript to build their voice profile. Uncertain matches stay
        unnamed.
      </p>
      {people.isPending ? (
        <Busy />
      ) : people.data?.people.length === 0 ? (
        <Empty
          title="No people yet"
          text="Add a person, then confirm their speaker label in a transcript."
        />
      ) : (
        <div className="people-list">
          {people.data?.people.map((p: any) => (
            <div className="person" key={p.id}>
              <div className="avatar">{p.name.charAt(0).toUpperCase()}</div>
              <div>
                <h2>{p.name}</h2>
                <p>
                  {p.samples} confirmed voice sample{p.samples !== 1 ? 's' : ''}
                </p>
              </div>
              <button
                aria-label={`Rename ${p.name}`}
                onClick={async () => {
                  const name = prompt('Name', p.name);
                  if (name?.trim()) {
                    try {
                      await api(`/v1/people/${p.id}`, {
                        method: 'PATCH',
                        body: JSON.stringify({ name }),
                      });
                      qc.invalidateQueries({ queryKey: ['people'] });
                    } catch (e) {
                      setActionError(e);
                    }
                  }
                }}
              >
                Rename
              </button>
              <button
                className="icon-button danger-text"
                aria-label={`Delete ${p.name} and voice profile`}
                onClick={async () => {
                  if (
                    confirm(
                      `Delete ${p.name} and their voice profile? Their transcript text is preserved.`,
                    )
                  ) {
                    try {
                      await api(`/v1/people/${p.id}`, { method: 'DELETE' });
                      qc.invalidateQueries({ queryKey: ['people'] });
                    } catch (e) {
                      setActionError(e);
                    }
                  }
                }}
              >
                <Trash2 size={16} />
              </button>
            </div>
          ))}
        </div>
      )}
    </>
  );
}
function Settings({ me }: { me: any }) {
  const qc = useQueryClient();
  const [actionError, setActionError] = useState<unknown>(null);
  const tokens = useQuery({ queryKey: ['tokens'], queryFn: () => api('/v1/tokens') });
  const [name, setName] = useState('');
  const [scope, setScope] = useState('upload');
  const [secret, setSecret] = useState('');
  const [copied, setCopied] = useState(false);
  const create = useMutation({
    mutationFn: () => api('/v1/tokens', { method: 'POST', body: JSON.stringify({ name, scope }) }),
    onSuccess: (v) => {
      setSecret(v.token);
      setName('');
      qc.invalidateQueries({ queryKey: ['tokens'] });
    },
  });
  return (
    <>
      <header className="page-header">
        <div>
          <h1>Settings</h1>
        </div>
        {me.auth_mode !== 'proxy' && (
          <button
            onClick={async () => {
              await api('/v1/logout', { method: 'POST' });
              qc.clear();
              location.assign('/');
            }}
          >
            <LogOut size={16} />
            Sign out
          </button>
        )}
      </header>
      <OAuthConnections publicUrl={me.public_url} />
      <section className="settings-section">
        <h2>Upload apps and access tokens</h2>
        <p>
          Enter this server address and an upload token in the native uploader. Read tokens remain
          available for clients that do not support OAuth.
        </p>
        <div className="server-url">
          <span>{me.public_url}</span>
          <button
            className="icon-button"
            aria-label="Copy server address"
            onClick={() =>
              navigator.clipboard
                .writeText(me.public_url)
                .catch(() =>
                  setActionError(
                    new Error('Copy is unavailable here. Select and copy the server address.'),
                  ),
                )
            }
          >
            <Copy size={16} />
          </button>
        </div>
        <form
          className="token-form"
          onSubmit={(e) => {
            e.preventDefault();
            create.mutate();
          }}
        >
          <label>
            Connection name
            <input
              placeholder="Mac uploader"
              value={name}
              onChange={(e) => setName(e.target.value)}
              required
              maxLength={100}
            />
          </label>
          <label>
            Access
            <select value={scope} onChange={(e) => setScope(e.target.value)}>
              <option value="upload">Upload recordings</option>
              <option value="read">Read recordings / MCP</option>
            </select>
          </label>
          <button className="primary" disabled={create.isPending}>
            <KeyRound size={16} />
            Create token
          </button>
        </form>
        <ErrorNotice error={actionError ?? create.error ?? tokens.error} />
        {secret && (
          <div className="notice secret">
            <strong>Copy this token now. It is shown only once.</strong>
            <code>{secret}</code>
            <button
              onClick={() => {
                navigator.clipboard
                  .writeText(secret)
                  .then(() => setCopied(true))
                  .catch(() => setActionError(new Error('Select and copy the displayed token.')));
              }}
            >
              {copied ? <Check size={15} /> : <Copy size={15} />}Copy token
            </button>
            <button
              onClick={() => {
                setSecret('');
                setCopied(false);
              }}
            >
              Done
            </button>
          </div>
        )}
        <div className="connections">
          {tokens.data?.tokens.map((t: any) => (
            <div className="connection" key={t.id}>
              <div>
                <strong>{t.name}</strong>
                <small>
                  {t.scope === 'read' ? 'Read / MCP' : 'Upload only'} · Created {date(t.created_at)}
                </small>
              </div>
              <button
                className="danger-text"
                onClick={async () => {
                  if (confirm(`Revoke access for ${t.name}?`)) {
                    try {
                      await api(`/v1/tokens/${t.id}`, { method: 'DELETE' });
                      qc.invalidateQueries({ queryKey: ['tokens'] });
                    } catch (e) {
                      setActionError(e);
                    }
                  }
                }}
              >
                Revoke
              </button>
            </div>
          ))}
        </div>
      </section>
      <section className="settings-section">
        <h2>MCP connection</h2>
        <p>
          Use a read token as the Bearer credential. The server exposes five read-only tools for
          browsing, searching, and reading transcript evidence.
        </p>
        <code className="endpoint">{me.public_url}mcp</code>
      </section>
      <section className="settings-section">
        <h2>Processing</h2>
        <dl>
          <div>
            <dt>Transcription</dt>
            <dd>OpenAI</dd>
          </div>
          <div>
            <dt>Local voice recognition</dt>
            <dd>
              {me.voice_configured ? 'Configured' : 'Configure the speaker model on the server'}
            </dd>
          </div>
          <div>
            <dt>Default recorder timezone</dt>
            <dd>{me.recorder_timezone}</dd>
          </div>
        </dl>
      </section>
    </>
  );
}
createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </QueryClientProvider>
  </React.StrictMode>,
);
