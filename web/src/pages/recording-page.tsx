import { useMemo, useState } from 'react';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { keepPreviousData, useQuery, useQueryClient } from '@tanstack/react-query';
import { FileAudio, RefreshCw, Trash2 } from 'lucide-react';
import {
  ApiError,
  api,
  isProcessing,
  peopleQuery,
  send,
  transcript,
  type Entry,
  type Person,
} from '../api';
import { dateTime, duration, speakerNames } from '../format';
import { useConfirm } from '../hooks/use-confirm';
import { usePlayback } from '../hooks/use-playback';
import { BackLink } from '../components/back-link';
import { ConfirmDialog } from '../components/confirm-dialog';
import { EditDetailsForm, type EntryEdit } from '../components/edit-details-form';
import { ErrorNotice } from '../components/error-notice';
import { LoadingState } from '../components/loading-state';
import { ProcessingNotice } from '../components/processing-notice';
import { SpeakerControl } from '../components/speaker-control';
import { TranscriptTurn } from '../components/transcript-turn';

export function RecordingPage() {
  const { id = '' } = useParams();
  const [params] = useSearchParams();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const confirm = useConfirm();
  const [editing, setEditing] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);

  const entry = useQuery({
    queryKey: ['entry', id],
    queryFn: () => api<Entry>(`/v1/entries/${id}`),
    refetchInterval: (q) => (q.state.data && isProcessing(q.state.data) ? 3000 : false),
  });
  const stage = entry.data?.stage;
  // Segments only change when processing moves on, so the stage is part of the key.
  const segments = useQuery({
    queryKey: ['transcript', id, stage],
    queryFn: ({ signal }) => transcript(id, signal),
    enabled: !!stage,
    placeholderData: keepPreviousData,
  });
  const people = useQuery(peopleQuery);
  const rows = useMemo(() => segments.data ?? [], [segments.data]);
  const names = useMemo(() => speakerNames(entry.data?.speakers), [entry.data?.speakers]);
  const playback = usePlayback(rows, Number(params.get('t')) * 1000 || 0);

  const refresh = () =>
    Promise.all(
      ['entry', 'entries', 'labels', 'search'].map((key) =>
        qc.invalidateQueries({ queryKey: key === 'entry' ? [key, id] : [key] }),
      ),
    );
  async function act(work: () => Promise<unknown>) {
    setActionError(null);
    setBusy(true);
    try {
      await work();
    } catch (e) {
      setActionError(e);
    } finally {
      setBusy(false);
    }
  }
  const post = (action: string) => () =>
    act(() => api(`/v1/entries/${id}/${action}`, send('POST')).then(refresh));
  const save = (edit: EntryEdit) =>
    act(async () => {
      await api(`/v1/entries/${id}`, send('PATCH', edit));
      await refresh();
      setEditing(false);
    });
  // Fails to the control, which shows the error beside it and keeps a typed name.
  // Names come from the entry's speakers, so the transcript itself is not refetched.
  const assign = (label: string) => async (personId: string | null, newName?: string) => {
    let person = personId;
    if (newName) person = (await api<Person>('/v1/people', send('POST', { name: newName }))).id;
    await api(
      `/v1/entries/${id}/speakers/${encodeURIComponent(label)}`,
      send('PATCH', { person_id: person }),
    );
    await Promise.all([
      qc.invalidateQueries({ queryKey: ['entry', id] }),
      qc.invalidateQueries({ queryKey: ['people'] }),
    ]);
  };

  if (entry.isPending) return <LoadingState label="Loading recording…" />;
  if (!entry.data)
    return (
      <>
        <BackLink />
        {entry.error instanceof ApiError && entry.error.status === 404 ? (
          <div className="notice">
            This recording is not in your library. It may be in Trash or deleted.
          </div>
        ) : (
          <ErrorNotice error={entry.error} />
        )}
      </>
    );
  const e = entry.data;
  const processing = isProcessing(e);

  return (
    <>
      <BackLink />
      <header className="detail-header">
        <div className="recording-meta">
          <time dateTime={e.recorded_at ?? undefined}>{dateTime(e.recorded_at)}</time>
          <span>{duration(e.duration_ms)}</span>
        </div>
        <h1>{e.title}</h1>
        {e.labels.length > 0 && (
          <div className="chips">
            {e.labels.map((l) => (
              <span key={l}>{l}</span>
            ))}
          </div>
        )}
        <div className="detail-actions">
          <button aria-expanded={editing} onClick={() => setEditing(!editing)}>
            Edit details
          </button>
          {!processing && rows.length > 0 && (
            <button disabled={busy} onClick={post('reanalyze')}>
              <RefreshCw size={14} aria-hidden />
              Reanalyze speakers
            </button>
          )}
          {['complete', 'failed', 'queued', 'paused'].includes(e.stage) && (
            <button
              className="danger"
              onClick={() =>
                confirm.ask({
                  title: 'Move this recording to Trash?',
                  text: 'It disappears from your library, search and chat apps. You can restore it from Trash.',
                  action: 'Move to Trash',
                  run: async () => {
                    await api(`/v1/entries/${id}/trash`, send('POST'));
                    await refresh();
                    navigate('/');
                  },
                })
              }
            >
              <Trash2 size={14} aria-hidden />
              Move to Trash
            </button>
          )}
        </div>
      </header>
      {editing && <EditDetailsForm entry={e} onSave={save} onCancel={() => setEditing(false)} />}
      <ErrorNotice error={actionError} />
      <ProcessingNotice entry={e} busy={busy} onPause={post('pause')} onResume={post('retry')} />
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
            {...playback.audioProps}
            controls
            preload="metadata"
            src={`/api/v1/entries/${id}/audio`}
          />
          <label>
            <input
              type="checkbox"
              checked={playback.follow}
              onChange={(x) => playback.setFollow(x.target.checked)}
            />
            Follow transcript
          </label>
        </div>
      )}
      <div className="detail-grid">
        <div className="detail-main">
          {e.summary && (
            <section className="summary">
              <div className="section-heading">
                <h2>Summary</h2>
                <span>Generated</span>
              </div>
              <p>{e.summary}</p>
            </section>
          )}
          <section className="transcript" aria-labelledby="transcript-heading">
            <div className="section-heading">
              <h2 id="transcript-heading">Transcript</h2>
              {rows.length > 0 && e.audio_available && <span>Select a time to listen</span>}
            </div>
            <ErrorNotice error={segments.error} />
            {rows.length === 0 &&
              !segments.error &&
              (segments.isFetching ? (
                <LoadingState label="Loading transcript…" />
              ) : (
                <p className="muted">
                  {processing
                    ? 'The transcript appears here when transcription finishes.'
                    : 'No speech was transcribed for this recording.'}
                </p>
              ))}
            {rows.map((r, i) => (
              <TranscriptTurn
                key={r.id}
                row={r}
                name={names.get(r.speaker) ?? r.speaker_name ?? 'Speaker'}
                showName={i === 0 || rows[i - 1].speaker !== r.speaker}
                active={i === playback.active}
                playable={e.audio_available}
                onSeek={playback.seek}
              />
            ))}
          </section>
        </div>
        <aside className="detail-side">
          {!!e.speakers?.length && (
            <section>
              <h2>Speakers</h2>
              {e.speakers.map((s) => (
                <SpeakerControl
                  key={s.label}
                  speaker={s}
                  name={names.get(s.label)!}
                  people={people.data?.people ?? []}
                  onAssign={assign(s.label)}
                />
              ))}
            </section>
          )}
          {!!e.sources?.length && (
            <section>
              <h2>Source files</h2>
              {e.sources.map((s, i) => (
                <div className="source" key={i}>
                  <FileAudio size={14} aria-hidden />
                  <span>{s.filename}</span>
                </div>
              ))}
            </section>
          )}
        </aside>
      </div>
      <ConfirmDialog {...confirm.dialog} />
    </>
  );
}
