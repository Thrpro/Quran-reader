import assert from 'node:assert/strict';
import test from 'node:test';
import { parseListeningPreferences, parseListeningResume } from '../src/listeningPreferences.ts';

test('listening preferences retain valid choices and reject unsafe persisted values', () => {
  const valid={mode:'surah',ayahReciterId:1001,reciterId:92,recordingId:100,speed:1.5,volume:.35,continuous:true};
  assert.deepEqual(parseListeningPreferences(JSON.stringify(valid)),valid);
  const fallback=parseListeningPreferences(null);
  assert.deepEqual(parseListeningPreferences('{invalid'),fallback);
  const invalid=parseListeningPreferences(JSON.stringify({mode:'script',ayahReciterId:-1,reciterId:1.5,recordingId:'2',speed:80,volume:-1,continuous:'yes'}));
  assert.deepEqual(invalid,fallback);
});

test('resume records require bounded references and positions without accepting arbitrary fields', () => {
  const raw={mode:'ayah',ayahReciterId:7,surah:112,ayah:2,seconds:4.5,untrusted:'ignored'};
  assert.equal(parseListeningResume(JSON.stringify(raw)).seconds,4.5);
  assert.equal('untrusted' in parseListeningResume(JSON.stringify(raw)),false);
  for(const patch of [{surah:0},{surah:115},{ayah:0},{ayah:287},{seconds:-1},{seconds:90000},{seconds:'4'}]) {
    assert.equal(parseListeningResume(JSON.stringify({...raw,...patch})),null);
  }
  assert.equal(parseListeningResume(null),null);
});
