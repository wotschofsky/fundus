import { useCallback, useEffect, useRef, useState, type SyntheticEvent } from 'react';
import type { Segment } from '../api';

/** Index of the last segment that has started at `ms` (segments are ordered by start). */
export function activeIndex(rows: Segment[], ms: number) {
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (rows[mid].start_ms <= ms) low = mid + 1;
    else high = mid;
  }
  return low - 1;
}

/**
 * Transcript-linked playback. State changes only when the spoken segment changes, so playback
 * re-renders two transcript rows, not all of them, however long the recording is.
 */
export function usePlayback(rows: Segment[], startMs: number) {
  const audio = useRef<HTMLAudioElement>(null);
  const [active, setActive] = useState(-1);
  const [playing, setPlaying] = useState(false);
  const [follow, setFollow] = useState(false);

  const seek = useCallback((ms: number) => {
    const player = audio.current;
    if (!player) return;
    player.currentTime = ms / 1000;
    player.play().catch(() => {});
  }, []);

  // A search link opens a passage: highlight and reveal it once, when the transcript arrives.
  // Later refetches (a speaker assignment, processing) leave the reader where they are.
  const revealed = useRef(0);
  useEffect(() => {
    if (!startMs || !rows.length || revealed.current === startMs) return;
    revealed.current = startMs;
    const index = activeIndex(rows, startMs);
    setActive(index);
    if (index >= 0) document.getElementById(rows[index].id)?.scrollIntoView({ block: 'center' });
  }, [startMs, rows]);

  useEffect(() => {
    if (!follow || !playing || active < 0) return;
    const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    document
      .getElementById(rows[active]?.id)
      ?.scrollIntoView({ block: 'center', behavior: still ? 'auto' : 'smooth' });
  }, [active, follow, playing, rows]);

  // Space toggles playback unless a control has focus; Shift+Space still scrolls up.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      const player = audio.current;
      if (event.key !== ' ' || !player) return;
      if (event.shiftKey || event.metaKey || event.ctrlKey || event.altKey) return;
      if (target.closest('input, textarea, select, button, a, audio, dialog, summary')) return;
      event.preventDefault();
      if (player.paused) player.play().catch(() => {});
      else player.pause();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const audioProps = {
    ref: audio,
    onTimeUpdate: (e: SyntheticEvent<HTMLAudioElement>) =>
      setActive(activeIndex(rows, e.currentTarget.currentTime * 1000)),
    onPlay: () => setPlaying(true),
    onPause: () => setPlaying(false),
    onLoadedMetadata: (e: SyntheticEvent<HTMLAudioElement>) => {
      if (startMs > 0) e.currentTarget.currentTime = startMs / 1000;
    },
  };
  return { audioProps, active, follow, setFollow, seek };
}
