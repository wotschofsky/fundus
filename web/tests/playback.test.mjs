import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { registerHooks, stripTypeScriptTypes } from 'node:module';
import test from 'node:test';
import { compileModule } from 'svelte/compiler';
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
globalThis.addEventListener ??= () => {};
globalThis.removeEventListener ??= () => {};
const { Playback } = await import('../src/playback.svelte.ts');

class Media extends EventTarget {
  currentTime = 0;
  playing = false;
  play() {
    this.playing = true;
    return Promise.resolve();
  }
}
const from = (media) => ({ currentTarget: media });

test('a player that replaces a playing one goes on from its place, whatever the old one still sends', () => {
  let playback;
  const stop = effect_root(() => {
    playback = new Playback(
      () => [],
      () => 0,
    );
  });
  const audio = new Media();
  playback.media = audio;
  flush();
  audio.currentTime = 42;
  playback.played(from(audio));
  playback.timeUpdate(from(audio));

  // The video becomes ready and replaces the audio while it plays.
  const video = new Media();
  playback.media = video;
  flush();
  // The removed audio's last timeupdate and its pause arrive a moment later.
  playback.timeUpdate(from(audio));
  playback.paused(from(audio));
  playback.loaded(from(video));

  assert.equal(video.currentTime, 42);
  assert.equal(video.playing, true);
  playback.played(from(video));
  assert.equal(playback.playing, true);
  stop();
});
