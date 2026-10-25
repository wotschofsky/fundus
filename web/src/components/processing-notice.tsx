import { LoaderCircle, Pause, Play } from 'lucide-react';
import { isProcessing, type Entry } from '../api';
import { stageName } from '../format';

/** Status of an unfinished recording, with Pause while it runs and Resume/Retry once stopped. */
export function ProcessingNotice({
  entry,
  busy,
  onPause,
  onResume,
}: {
  entry: Entry;
  busy: boolean;
  onPause: () => void;
  onResume: () => void;
}) {
  if (entry.stage === 'complete') return null;
  const processing = isProcessing(entry);
  return (
    <div className={`notice status ${entry.stage === 'failed' ? 'error' : ''}`} role="status">
      <div>
        <strong>
          {processing && <LoaderCircle className="spin" size={14} aria-hidden />}
          {stageName(entry)}
        </strong>
        <p>
          {entry.failure ??
            (processing
              ? 'Your server is processing this recording. You can leave this page.'
              : 'Processing is paused. Resume to continue where it stopped.')}
        </p>
      </div>
      {processing ? (
        <button disabled={busy || entry.pause_requested} onClick={onPause}>
          <Pause size={14} aria-hidden />
          Pause
        </button>
      ) : (
        <button disabled={busy} onClick={onResume}>
          <Play size={14} aria-hidden />
          {entry.stage === 'failed' ? 'Retry' : 'Resume'}
        </button>
      )}
    </div>
  );
}
