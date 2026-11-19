import assert from 'node:assert/strict';
import test from 'node:test';
import { speakerNames, startAt } from '../src/format.ts';

test("a transcript's own speaker names win over the recording's", () => {
  // The recording was read before the owner renamed speaker A and cleared B's name; its
  // transcript after. C has no turns in it; D is a speaker the recording did not list yet.
  const speakers = [
    { label: 'A', name: 'Alex' },
    { label: 'B', name: 'Morgan' },
    { label: 'C', name: null },
  ];
  const turns = [
    { speaker: 'A', speaker_name: 'Robin' },
    { speaker: 'B', speaker_name: null },
    { speaker: 'D', speaker_name: null },
  ];
  const names = speakerNames(speakers, turns);
  assert.equal(names.get('A'), 'Robin');
  assert.equal(names.get('B'), 'Speaker 2');
  assert.equal(names.get('C'), 'Speaker 3');
  assert.equal(names.get('D'), undefined);
  // Without the transcript, the recording's names stand and unnamed speakers are numbered.
  assert.equal(speakerNames(speakers).get('A'), 'Alex');
  assert.equal(speakerNames(speakers).get('C'), 'Speaker 3');
});

test('a link to a moment starts there only for a time a player can seek to', () => {
  assert.equal(startAt('12.5'), 12_500);
  assert.equal(startAt(null), 0);
  assert.equal(startAt('Infinity'), 0);
  assert.equal(startAt('-5'), 0);
  assert.equal(startAt('soon'), 0);
  // Finite in seconds, but not in milliseconds.
  assert.equal(startAt('1e308'), 0);
});
