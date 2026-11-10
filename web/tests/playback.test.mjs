import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { registerHooks, stripTypeScriptTypes } from 'node:module';
import test from 'node:test';
import { compileModule } from 'svelte/compiler';
// Svelte's client runtime, as the compiled module uses it: under Node, `svelte` itself is the
// server build, and an effect outside a component needs a root (Svelte is pinned exactly).
import { effect_root, flush } from 'svelte/internal/client';

// A `.svelte.ts` module uses runes, so it is compiled as Vite would before it is imported, and
// its imports leave out `.ts` as Vite allows.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (error) {
      if (!specifier.startsWith('.')) throw error;
      return next(`${specifier}.ts`, context);
    }
  },
  load(url, context, next) {
    if (!url.endsWith('.svelte.ts')) return next(url, context);
    const source = stripTypeScriptTypes(readFileSync(new URL(url), 'utf8'));
    const { js } = compileModule(source, { filename: url, generate: 'client' });
    return { format: 'module', source: js.code, shortCircuit: true };
  },
});
// The Space shortcut listens on the window, which Node does not have.
globalThis.addEventListener ??= () => {};
globalThis.removeEventListener ??= () => {};
const { Playback } = await import('../src/playback.svelte.ts');

class Media extends EventTarget {
  currentTime = 0;
  readyState = 4;
  playing = false;
  play() {
    this.playing = true;
    return Promise.resolve();
  }
}
const from = (media) => ({ currentTarget: media });

/** A page with its audio at 42 s, playing or not. Each swap flushes, as Svelte does between a
 * browser's events. */
function playbackAt42(playing) {
  let playback;
  const stop = effect_root(() => {
    playback = new Playback(
      () => [],
      () => 0,
    );
  });
  const audio = new Media();
  const show = (media) => {
    playback.media = media;
    flush();
  };
  show(audio);
  audio.currentTime = 42;
  if (playing) playback.played(from(audio));
  playback.timeUpdate(from(audio));
  flush();
  return { playback, audio, show, stop };
}

test('a player that replaces a playing one goes on from its place, whatever the old one still sends', () => {
  const { playback, audio, show, stop } = playbackAt42(true);
  // The video becomes ready and replaces the audio.
  const video = new Media();
  show(video);
  // The removed audio's last timeupdate arrives a moment later.
  playback.timeUpdate(from(audio));
  playback.loaded(from(video));
  assert.equal(video.currentTime, 42);
  assert.equal(video.playing, true);
  // Its pause, should it come later still, does not stop the one playing now. (The video's
  // play event, which the fake's play() does not send.)
  playback.played(from(video));
  playback.paused(from(audio));
  assert.equal(playback.playing, true);
  stop();
});

test('after a video that could not load, the audio goes on from its place and keeps playing', () => {
  const { playback, audio, show, stop } = playbackAt42(true);
  // Try again: the video replaces the audio, fails before it loads, and the audio comes back.
  const video = new Media();
  show(video);
  playback.paused(from(audio));
  const again = new Media();
  show(again);
  playback.loaded(from(again));
  assert.equal(again.currentTime, 42);
  assert.equal(again.playing, true);
  stop();
});

test('a player that replaces one just sent elsewhere starts there', () => {
  const { playback, show, stop } = playbackAt42(true);
  // A transcript time is clicked, and the video replaces the audio before its next timeupdate.
  playback.seek(30_000);
  const video = new Media();
  show(video);
  playback.loaded(from(video));
  assert.equal(video.currentTime, 30);
  stop();
});

test('a replacing player paused before it loaded stays paused at the place', () => {
  const { playback, show, stop } = playbackAt42(true);
  // Play and pause on the video while it is still loading: it is at 0 and sends that.
  const video = new Media();
  video.readyState = 0;
  show(video);
  playback.played(from(video));
  playback.timeUpdate(from(video));
  playback.paused(from(video));
  video.readyState = 1;
  playback.loaded(from(video));
  assert.equal(video.currentTime, 42);
  assert.equal(video.playing, false);
  stop();
});

test('a paused player that is replaced stays paused at its place', () => {
  const { playback, show, stop } = playbackAt42(false);
  const video = new Media();
  show(video);
  playback.loaded(from(video));
  assert.equal(video.currentTime, 42);
  assert.equal(video.playing, false);
  stop();
});
