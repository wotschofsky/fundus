/** A temporary playback boundary on the shared player; ordinary playback owns no listeners. */
export class SnippetPlayback {
  #audio: HTMLMediaElement | undefined;
  #end = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #run = 0;
  readonly #updates = ['playing', 'timeupdate', 'seeked', 'ratechange'];
  readonly #pauses = ['pause', 'waiting', 'stalled'];

  play(audio: HTMLMediaElement, startMs: number, endMs: number) {
    this.stop();
    if (endMs <= startMs) return;
    this.#audio = audio;
    this.#end = endMs / 1000;
    const run = this.#run;
    for (const event of this.#updates) audio.addEventListener(event, this.#schedule);
    for (const event of this.#pauses) audio.addEventListener(event, this.#clearTimer);
    audio.addEventListener('ended', this.stop);
    audio.currentTime = startMs / 1000;
    audio.play().catch(() => {
      if (this.#run === run) this.stop();
    });
    this.#schedule();
  }

  #clearTimer = () => {
    clearTimeout(this.#timer);
    this.#timer = undefined;
  };

  #schedule = () => {
    this.#clearTimer();
    const audio = this.#audio;
    if (!audio) return;
    if (audio.currentTime >= this.#end) {
      this.#finish();
      return;
    }
    if (audio.paused || audio.seeking || audio.readyState < 3 || audio.playbackRate <= 0) return;
    // timeupdate alone can arrive several hundred milliseconds after the next passage starts.
    // Recheck the actual media clock when this timer fires: buffering must not end a clip early.
    const remaining = (this.#end - audio.currentTime) / audio.playbackRate;
    this.#timer = setTimeout(this.#schedule, Math.max(10, remaining * 1000));
  };

  #finish() {
    const audio = this.#audio;
    const end = this.#end;
    this.stop();
    if (audio) audio.currentTime = end;
  }

  stop = () => {
    this.#run++;
    this.#clearTimer();
    const audio = this.#audio;
    this.#audio = undefined;
    if (!audio) return;
    for (const event of this.#updates) audio.removeEventListener(event, this.#schedule);
    for (const event of this.#pauses) audio.removeEventListener(event, this.#clearTimer);
    audio.removeEventListener('ended', this.stop);
    audio.pause();
  };
}
