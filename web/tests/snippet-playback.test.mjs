import assert from 'node:assert/strict';
import { once } from 'node:events';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { SnippetPlayback } from '../src/snippet-playback.ts';

// A moving media clock: tests exercise real scheduled stops, rather than invoking private helpers.
class Media extends EventTarget {
  paused = true;
  seeking = false;
  readyState = 4;
  #time = 0;
  #at = performance.now();
  #rate = 1;
  get currentTime() {
    return (
      this.#time +
      (this.paused || this.readyState < 3
        ? 0
        : ((performance.now() - this.#at) / 1000) * this.#rate)
    );
  }
  set currentTime(time) {
    this.#time = time;
    this.#at = performance.now();
    this.dispatchEvent(new Event('seeked'));
  }
  get playbackRate() {
    return this.#rate;
  }
  set playbackRate(rate) {
    this.#time = this.currentTime;
    this.#at = performance.now();
    this.#rate = rate;
    this.dispatchEvent(new Event('ratechange'));
  }
  play() {
    this.#at = performance.now();
    this.paused = false;
    this.dispatchEvent(new Event('playing'));
    return Promise.resolve();
  }
  pause() {
    this.#time = this.currentTime;
    this.paused = true;
    this.dispatchEvent(new Event('pause'));
  }
  buffer() {
    this.#time = this.currentTime;
    this.readyState = 2;
    this.dispatchEvent(new Event('waiting'));
  }
  resume() {
    this.readyState = 4;
    this.#at = performance.now();
    this.dispatchEvent(new Event('playing'));
  }
}

test('a snippet starts at its offset and automatically stops at its end', async () => {
  const audio = new Media();
  const snippets = new SnippetPlayback();
  snippets.play(audio, 1000, 1040);
  assert.ok(audio.currentTime >= 1 && audio.currentTime < 1.04);
  await once(audio, 'pause');
  assert.equal(audio.paused, true);
  assert.equal(audio.currentTime, 1.04);
});

test('buffering does not end the snippet early, and playback speed changes are honored', async () => {
  const audio = new Media();
  const snippets = new SnippetPlayback();
  snippets.play(audio, 1000, 1080);
  audio.buffer();
  await delay(100);
  assert.equal(audio.paused, false);
  assert.ok(audio.currentTime < 1.08);
  audio.playbackRate = 2;
  audio.resume();
  await once(audio, 'pause');
  assert.equal(audio.currentTime, 1.08);
});

test('switching samples replaces the former boundary', async () => {
  const audio = new Media();
  const snippets = new SnippetPlayback();
  snippets.play(audio, 0, 1000);
  snippets.play(audio, 2000, 2040);
  await once(audio, 'pause');
  assert.equal(audio.currentTime, 2.04);
});

test('stopping a preview removes the boundary for normal continuous playback', async () => {
  const audio = new Media();
  const snippets = new SnippetPlayback();
  snippets.play(audio, 0, 30);
  snippets.stop();
  await audio.play();
  await delay(60);
  assert.equal(audio.paused, false);
  assert.ok(audio.currentTime > 0.03);
  audio.pause();
});

test('a late play rejection from an old sample cannot cancel its replacement', async () => {
  const audio = new Media();
  const snippets = new SnippetPlayback();
  let rejectOld;
  const play = audio.play.bind(audio);
  audio.play = () =>
    new Promise((_, reject) => {
      rejectOld = reject;
    });
  snippets.play(audio, 0, 1000);
  audio.play = play;
  snippets.play(audio, 2000, 2040);
  rejectOld(new Error('Old playback was interrupted'));
  await Promise.resolve();
  assert.equal(audio.paused, false);
  await once(audio, 'pause');
  assert.equal(audio.currentTime, 2.04);
});
