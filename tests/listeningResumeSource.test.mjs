import assert from 'node:assert/strict';
import test from 'node:test';
import { resumeForSource } from '../src/listeningResumeSource.ts';

const source = { mode:'ayah', ayahReciterId:1001, reciterId:0, recordingId:0,
  surah:112, ayah:2, timedRecording:true };
const selection = { ...source, speed:1.25, volume:.35, continuous:false };

test('resume saves an offset only for the attached performance and point', () => {
  const result = resumeForSource(selection, source, 4.5);
  assert.equal(result.seconds,4.5);
  assert.equal(result.surah,112);
  assert.equal(result.ayah,2);
  assert.equal(result.speed,1.25);
  assert.equal('timedRecording' in result,false);
  assert.equal(resumeForSource(selection,null,4.5),null);
  for (const change of [{ayahReciterId:7},{surah:113},{ayah:3},{mode:'surah'},{timedRecording:false}]) {
    assert.equal(resumeForSource({...selection,...change},source,4.5),null);
  }
});

test('additional recordings cannot inherit another reciter or performance offset', () => {
  const additional = {...source,mode:'surah',reciterId:92,recordingId:100,timedRecording:false};
  const current = {...selection,...additional};
  assert.equal(resumeForSource(current,additional,123).seconds,123);
  assert.equal(resumeForSource({...current,reciterId:93},additional,123),null);
  assert.equal(resumeForSource({...current,recordingId:101},additional,123),null);
});

test('follow-along updates may save the new point but clip offsets remain zero', () => {
  const next = {...source,surah:113,ayah:1};
  assert.equal(resumeForSource({...selection,...next},source,1),null);
  assert.equal(resumeForSource({...selection,...next},next,1).surah,113);
  const clips = {...source,timedRecording:false};
  assert.equal(resumeForSource({...selection,...clips},clips,123).seconds,0);
});

test('resume rejects invalid offsets and keeps the latest safe playback preferences', () => {
  for(const seconds of [NaN,Infinity,-1,86401]) assert.equal(resumeForSource(selection,source,seconds),null);
  const result=resumeForSource({...selection,speed:1.5,volume:.5,continuous:true},source,0);
  assert.equal(result.seconds,0);
  assert.equal(result.speed,1.5);
  assert.equal(result.volume,.5);
  assert.equal(result.continuous,true);
});
