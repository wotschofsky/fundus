import assert from 'node:assert/strict';
import test from 'node:test';
import { speakerNames, startAt } from '../src/format.ts';

test("a transcript's own speaker names win over the recording's", () => {
  // The recording was read before the owner renamed speaker A and cleared B's name; its
  // transcript after. C has no turns in it; D and E are speakers the recording did not list yet
  // (the transcript was saved between the two reads).
  const speakers = [
    { label: 'A', name: 'Alex' },
    { label: 'B', name: 'Morgan' },
    { label: 'C', name: null },
  ];
  const turns = [
    { speaker: 'A', speaker_name: 'Robin' },
    { speaker: 'B', speaker_name: null },
    { speaker: 'E', speaker_name: null },
    { speaker: 'D', speaker_name: null },
    { speaker: 'E', speaker_name: null },
  ];
  const names = speakerNames(speakers, turns);
  assert.equal(names.get('A'), 'Robin');
  assert.equal(names.get('B'), 'Speaker 2');
  assert.equal(names.get('C'), 'Speaker 3');
  // Numbered after the listed ones in label order, as the recording will list them.
  assert.equal(names.get('D'), 'Speaker 4');
  assert.equal(names.get('E'), 'Speaker 5');
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

test('a speaker the recording does not list yet keeps its number once the list has it', () => {
  // The recording lists A; its transcript, saved after it was read, adds C and then B (named).
  const turns = [
    { speaker: 'A', speaker_name: null },
    { speaker: 'C', speaker_name: null },
    { speaker: 'B', speaker_name: 'Robin' },
  ];
  const before = speakerNames([{ label: 'A', name: null }], turns);
  const after = speakerNames(
    [
      { label: 'A', name: null },
      { label: 'B', name: 'Robin' },
      { label: 'C', name: null },
    ],
    turns,
  );
  assert.equal(before.get('B'), 'Robin');
  assert.equal(before.get('C'), after.get('C'));
  assert.equal(after.get('C'), 'Speaker 3');
});
