import type { Segment } from './api';
import { SnippetPlayback } from './snippet-playback';

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
 * Transcript-linked playback. `active` changes only when the spoken segment changes, so
 * playback touches two transcript rows, not all of them, however long the recording is.
 * Create it while a component initializes: it registers effects.
 */
export class Playback {
  /** The audio or video player. */
  media = $state<HTMLMediaElement>();
  active = $state(-1);
  playing = $state(false);
  follow = $state(false);
  #revealed = 0;
  #rows: () => Segment[];
  #startMs: () => number;
  #snippet = new SnippetPlayback();

  constructor(rows: () => Segment[], startMs: () => number) {
    this.#rows = rows;
    this.#startMs = startMs;
    $effect(() => {
      if (!this.media) return;
      return () => this.#snippet.stop();
    });
    // A search link opens a passage: highlight and reveal it once, when the transcript arrives.
    // Later refetches (a speaker assignment, processing) leave the reader where they are.
    $effect(() => {
      const list = rows();
      const start = startMs();
      if (!start || !list.length || this.#revealed === start) return;
      this.#revealed = start;
      const index = activeIndex(list, start);
      this.active = index;
      // A frame later, so the router restoring a Back position does not scroll it away again.
      const id = list[index]?.id;
      if (id)
        requestAnimationFrame(() =>
          document.getElementById(id)?.scrollIntoView({ block: 'center' }),
        );
    });

    $effect(() => {
      if (!this.follow || !this.playing || this.active < 0) return;
      const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
      document
        .getElementById(rows()[this.active]?.id)
        ?.scrollIntoView({ block: 'center', behavior: still ? 'auto' : 'smooth' });
    });

    // Space toggles playback unless a control has focus; Shift+Space still scrolls up.
    $effect(() => {
      const onKey = (event: KeyboardEvent) => {
        const target = event.target as HTMLElement;
        const player = this.media;
        if (event.key !== ' ' || !player) return;
        if (event.shiftKey || event.metaKey || event.ctrlKey || event.altKey) return;
        if (target.closest('input, textarea, select, button, a, audio, video, dialog, summary'))
          return;
        event.preventDefault();
        if (player.paused) player.play().catch(() => {});
        else player.pause();
      };
      addEventListener('keydown', onKey);
      return () => removeEventListener('keydown', onKey);
    });
  }

  timeUpdate = () => {
    if (this.media) this.active = activeIndex(this.#rows(), this.media.currentTime * 1000);
  };

  /** Starts at the linked passage. */
  loaded = () => {
    if (this.media && this.#startMs() > 0) this.media.currentTime = this.#startMs() / 1000;
  };

  seek = (ms: number) => {
    this.#snippet.stop();
    if (!this.media) return;
    this.media.currentTime = ms / 1000;
    this.media.play().catch(() => {});
  };

  listen = (segment: Pick<Segment, 'start_ms' | 'end_ms'>) => {
    if (this.media) this.#snippet.play(this.media, segment.start_ms, segment.end_ms);
  };

  stopListening = () => this.#snippet.stop();
}
