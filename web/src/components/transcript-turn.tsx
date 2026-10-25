import { memo } from 'react';
import type { Segment } from '../api';
import { timestamp } from '../format';

/** One transcript segment. Memoized: playback only re-renders the rows whose `active` flips. */
export const TranscriptTurn = memo(function TranscriptTurn({
  row,
  name,
  showName,
  active,
  playable,
  onSeek,
}: {
  row: Segment;
  name: string;
  showName: boolean;
  active: boolean;
  playable: boolean;
  onSeek: (ms: number) => void;
}) {
  return (
    <article
      className={`turn${active ? ' active' : ''}${showName ? '' : ' continued'}`}
      id={row.id}
    >
      {playable ? (
        <button
          className="time"
          onClick={() => onSeek(row.start_ms)}
          aria-label={`Play from ${timestamp(row.start_ms)}`}
        >
          {timestamp(row.start_ms)}
        </button>
      ) : (
        <span className="time">{timestamp(row.start_ms)}</span>
      )}
      <div>
        {showName && <strong>{name}</strong>}
        <p>{row.text}</p>
      </div>
    </article>
  );
});
