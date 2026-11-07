import assert from 'node:assert/strict';
import test from 'node:test';
import { pageFonts, ReaderFontLoader } from '../src/readerFonts.ts';

test('opening pages wait only for their own supplied fonts',()=>{
  assert.deepEqual(pageFonts([{lines:[{type:'ayah'}]}]),['text']);
  assert.deepEqual(pageFonts([{lines:[{type:'surah_name'},{type:'basmallah'}]}]),['text','header','basmala']);
  assert.deepEqual(pageFonts([{lines:[{type:'surah_name'}]}]),['text','header']);
});
test('shared font requests load once and successful faces stay ready',async()=>{
  let calls=0,release;
  const loader=new ReaderFontLoader(()=>{calls++;return new Promise(done=>release=done);});
  const first=loader.load(['text']),second=loader.load(['text']);
  await Promise.resolve();assert.equal(calls,1);assert.equal(loader.ready(['text']),false);
  release();await Promise.all([first,second]);assert.equal(loader.ready(['text']),true);
  await loader.load(['text']);assert.equal(calls,1);
});
test('font failure is never accepted as ready and can be retried',async()=>{
  let calls=0;
  const loader=new ReaderFontLoader(async()=>{if(++calls===1)throw Error('temporary');});
  await assert.rejects(loader.load(['header']),/temporary/);assert.equal(loader.ready(['header']),false);
  await loader.load(['header']);assert.equal(loader.ready(['header']),true);assert.equal(calls,2);
});
test('pending decorative faces do not block an already loaded text face',async()=>{
  let release;const loader=new ReaderFontLoader(declaration=>declaration.includes('Header')?new Promise(done=>release=done):Promise.resolve());
  await loader.load(['text']);const decorative=loader.load(['header']);await Promise.resolve();
  assert.equal(loader.ready(['text']),true);assert.equal(loader.ready(['text','header']),false);
  release();await decorative;assert.equal(loader.ready(['text','header']),true);
});
