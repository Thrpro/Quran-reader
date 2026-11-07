import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {groupAyahWords, pageAyahScope, scopedHighlight, scopedRecitation, wordAyahScope} from '../src/ayahRenderScope.ts';

test('scope retains exact fragment membership, empty lines and the original active selection identity',()=>{
  const words=[{id:10,text:'a',surah:2,ayah:1},{id:13,text:'b',surah:2,ayah:2}];
  const scope=wordAyahScope(words),selection={firstWordId:13,lastWordId:13};
  assert.equal(wordAyahScope(words),scope,'immutable source reuses its scope');
  assert.equal(scopedHighlight(scope,selection),selection);
  assert.equal(scopedHighlight(scope,{firstWordId:11,lastWordId:12}),null,'a missing word between source fragments is not selected');
  assert.equal(scopedHighlight(scope,{firstWordId:1,lastWordId:9}),null);
  assert.equal(scopedHighlight(wordAyahScope([]),selection),null);
  assert.equal(scopedRecitation(scope,'2:1'),'2:1');
  assert.equal(scopedRecitation(scope,'2:3'),null);
  assert.equal(scopedRecitation(scope,null),null);
});

test('a replacement source page gets a new index without stale ayah membership',()=>{
  const first={lines:[{words:[{id:1,text:'a',surah:1,ayah:1}]}]};
  const replacement={lines:[{words:[{id:2,text:'b',surah:1,ayah:2}]}]};
  assert.equal(pageAyahScope(first),pageAyahScope(first));
  assert.notEqual(pageAyahScope(first),pageAyahScope(replacement));
  assert.equal(scopedRecitation(pageAyahScope(replacement),'1:1'),null);
  assert.equal(scopedRecitation(pageAyahScope(replacement),'1:2'),'1:2');
});

test('all 604 source pages preserve ordered words and cross-line/surah ayah scopes',async()=>{
  let lineCount=0,wordCount=0;
  for(let number=1;number<=604;number++) {
    const page=JSON.parse(await readFile(`public-qul/experimental/digital-khatt/pages/p${number}.json`,'utf8'));
    const scope=pageAyahScope(page);
    for(const line of page.lines) {
      const groups=groupAyahWords(line.words),lineScope=wordAyahScope(line.words);
      assert.deepEqual(groups.flatMap(group=>group.words),line.words,`ordered original words on page ${number} line ${line.lineNumber}`);
      assert.equal(groups.flatMap(group=>group.words).map(word=>word.text).join(' '),line.words.map(word=>word.text).join(' '));
      for(const word of line.words) {
        const key=`${word.surah}:${word.ayah}`,selection={firstWordId:word.id,lastWordId:word.id};
        assert.equal(scopedRecitation(scope,key),key);assert.equal(scopedRecitation(lineScope,key),key);
        assert.equal(scopedHighlight(scope,selection),selection);assert.equal(scopedHighlight(lineScope,selection),selection);
        wordCount++;
      }
      assert.equal(scopedRecitation(lineScope,'115:1'),null);
      lineCount++;
    }
  }
  assert.ok(lineCount>8500);assert.ok(wordCount>70000);
});
