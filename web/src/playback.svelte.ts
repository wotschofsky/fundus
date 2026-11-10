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
  /** Where a player that replaces another (the video once it is ready, the audio after a video
   * that failed) goes on, and whether it plays: a player went while playing since the last one
   * loaded, and nobody paused since. */
  #position: number | null = null;
  #resume = false;
  #rows: () => Segment[];
  #startMs: () => number;
  #snippet = new SnippetPlayback();

  constructor(rows: () => Segment[], startMs: () => number) {
    this.#rows = rows;
    this.#startMs = startMs;
    $effect(() => {
      if (!this.media) return;
      return () => {
        // Only ever set here, so a player that went again before it loaded (a video this browser
        // cannot play) leaves it as the players before it did; `loaded` and a pause clear it.
        this.#resume ||= this.playing;
        this.playing = false;
        this.#snippet.stop();
      };
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

  /** Only the current player's events count: a removed one still sends its last timeupdate and
   * its pause a moment after it goes, and those must not move the one that replaced it. */
  #current = (event: Event) => event.currentTarget === this.media;

  timeUpdate = (event: Event) => {
    // A player that has not loaded yet is at 0, which is not the place.
    if (!this.#current(event) || !this.media?.readyState) return;
    this.#position = this.media.currentTime;
    this.active = activeIndex(this.#rows(), this.#position * 1000);
  };

  played = (event: Event) => {
    if (this.#current(event)) this.playing = true;
  };

  paused = (event: Event) => {
    if (!this.#current(event)) return;
    this.playing = false;
    this.#resume = false;
  };

  /** Starts where the previous player was, or at the linked passage. */
  loaded = (event: Event) => {
    if (!this.#current(event) || !this.media) return;
    const start = this.#position ?? this.#startMs() / 1000;
    if (start > 0) this.media.currentTime = start;
    if (this.#resume) this.media.play().catch(() => {});
    this.#resume = false;
  };

  seek = (ms: number) => {
    this.#snippet.stop();
    if (!this.media) return;
    // Also the place for a player that replaces this one before its next timeupdate.
    this.#position = this.media.currentTime = ms / 1000;
    this.media.play().catch(() => {});
  };

  listen = (segment: Pick<Segment, 'start_ms' | 'end_ms'>) => {
    if (this.media) this.#snippet.play(this.media, segment.start_ms, segment.end_ms);
  };

  stopListening = () => this.#snippet.stop();
}
