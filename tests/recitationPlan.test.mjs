import test from 'node:test';
import assert from 'node:assert/strict';
import { recitationPlan } from '../src/recitationPlan.ts';
import { AYAH_RECITERS } from '../src/audioCatalog.ts';
const surahs=[{id:1,versesCount:7},{id:8,versesCount:75},{id:9,versesCount:129},{id:10,versesCount:109},{id:112,versesCount:4}];
test('basmala precedes surah openings in the same voice, without duplicating Al-Fatihah or adding it to At-Tawbah',()=>{
 for(const reciter of AYAH_RECITERS){
 const full=recitationPlan(reciter,{surah:112,ayah:1},{surah:112,ayah:4},surahs);
 assert.equal(full.length,5);assert.equal(full[0].basmala,true);assert.ok(full[0].url.endsWith('/001001.mp3'));
 assert.deepEqual(full.slice(1).map(s=>s.ayah),[1,2,3,4]);
 assert.ok(!recitationPlan(reciter,{surah:1,ayah:1},{surah:1,ayah:7},surahs).some(s=>s.basmala));
 assert.ok(!recitationPlan(reciter,{surah:9,ayah:1},{surah:9,ayah:2},surahs).some(s=>s.basmala));
 assert.ok(!recitationPlan(reciter,{surah:112,ayah:2},{surah:112,ayah:4},surahs).some(s=>s.basmala));
 }
});
test('ranges cross chapters without skipping, repeating, or running past the requested ayah',()=>{
 const plan=recitationPlan(AYAH_RECITERS[0],{surah:8,ayah:75},{surah:10,ayah:2},surahs);
 assert.deepEqual(plan.filter(s=>s.basmala).map(s=>s.surah),[10]);assert.equal(plan[0].ayah,75);assert.deepEqual(plan.at(-1),{surah:10,ayah:2,url:plan.at(-1).url});assert.equal(plan.filter(s=>!s.basmala).length,132);
});
