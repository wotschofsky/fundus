import assert from 'node:assert/strict';
import test from 'node:test';
import { speakerNames } from '../src/format.ts';

test("a transcript's own speaker names win over the recording's", () => {
  // The recording was read before the owner named speaker A and cleared B's name; its
  // transcript after.
  const speakers = [
    { label: 'A', name: null },
    { label: 'B', name: 'Morgan' },
  ];
  const turns = [
    { speaker: 'A', speaker_name: 'Robin' },
    { speaker: 'B', speaker_name: null },
    { speaker: 'C', speaker_name: null },
  ];
  const names = speakerNames(speakers, turns);
  assert.equal(names.get('A'), 'Robin');
  assert.equal(names.get('B'), 'Speaker 2');
  assert.equal(names.get('C'), undefined);
  // Without the transcript, unnamed speakers are numbered.
  assert.equal(speakerNames(speakers).get('A'), 'Speaker 1');
});
