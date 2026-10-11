import test from 'node:test';
import assert from 'node:assert/strict';
import {existsSync, readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';

test('all shaped pages match immutable source data and valid word/glyph mappings',(context)=>{
  const base='public-qul/experimental/digital-khatt/';
  if (!existsSync(base + 'shaped/manifest.json')) {
    assert.fail('Run npm run shape:pages: the reader requires verified shaped pages');
  }
  const read=p=>JSON.parse(readFileSync(base+p,'utf8'));
  const manifest=read('shaped/manifest.json');
  assert.equal(manifest.version, 2, 'Rebuild shaped pages with the current shaper');
  for (const [path, hash] of Object.entries(manifest.toolHashes)) {
    assert.equal(createHash('sha256').update(readFileSync(path)).digest('hex'), hash, 'Stale shaping tool: ' + path);
  }
  assert.equal(Object.keys(manifest.toolHashes).length, 6);
  const paths=read('shaped/glyphs.json');
  let count=0;
  for(let n=1;n<=604;n++) {
    const raw=readFileSync(base+`pages/p${n}.json`);
    assert.equal(createHash('sha256').update(raw).digest('hex'),manifest.sourceHashes[n],`Stale shape: page ${n}`);
    const source=JSON.parse(raw), shape=read(`shaped/p${n}.json`);
    assert.equal(shape.pageNumber,n);
    for(const line of source.lines) {
      const s=shape.lines[line.lineNumber];
      if(line.type!=='ayah'||line.isCentered) {assert.equal(s,undefined);continue;}
      count++;
      assert.ok(s && s.glyphs.length>0);
      assert.ok(s.width>0 && Number.isFinite(s.scale) && s.scale>0);
      assert.ok(Math.abs(s.bounds[0]*s.scale+s.shift+s.width)<0.01);
      assert.ok(Math.abs(s.bounds[2]*s.scale+s.shift)<0.01);
      const ids=new Set(line.words.map(w=>w.id));
      assert.deepEqual(new Set(s.glyphs.map(g=>g[3])),ids,`Unrendered source word on page ${n}, line ${line.lineNumber}`);
      for (const id of ids) {
        assert.ok(s.wordBounds[id]?.every(Number.isFinite), `Missing hit bounds for word ${id}`);
        assert.ok(s.wordBounds[id][1] > s.wordBounds[id][0]);
      }
      for(const [id,x,y,word] of s.glyphs) {
        assert.ok(id>0 && typeof paths[id]==='string');
        assert.ok(Number.isFinite(x)&&Number.isFinite(y));
        assert.ok(ids.has(word));
      }
    }
  }
  assert.equal(count,manifest.lineCount);
});
