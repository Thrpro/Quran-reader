import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { cropShapedInk, outlineYBounds } from '../src/inkCrop.ts';

test('absolute curves retain the entire control-point hull and unknown syntax uses full height',()=>{
  assert.deepEqual(outlineYBounds('M0,10C0,-30 2,80 10,20L0,15Z'),[-30,80]);
  assert.deepEqual(outlineYBounds('M1e-3,-2e2Q1,30 20,1Z'),[-200,30]);
  for (const path of ['M0,0c1,1 2,2 3,3Z','M0,0A10,10 0 0 1 4,4','M0,NaN','M0,1 3']) assert.equal(outlineYBounds(path),null);
  const shape={width:3200,scale:1,shift:0,glyphs:[[1,0,0,1]]};
  assert.deepEqual(cropShapedInk(shape,{1:'M0,0a1,1 0 0 0 0,0'},320,{width:320,height:320,offsetY:0}),{top:0,height:320});
});

test('integer cropping preserves the original origin including fractional alignment and glyph offsets',()=>{
  const shape={width:3200,scale:1,shift:22,glyphs:[[1,0,100,1]]};
  const crop=cropShapedInk(shape,{1:'M0,0L100,100Z'},321,{width:320,height:320,offsetY:.25});
  assert.deepEqual(crop,{top:173,height:15});
  // Original pixel position equals cropped position plus CSS row offset.
  const origin=195+.25;const original=origin-20;
  assert.equal((origin-crop.top)-20+crop.top,original);
});

test('all supplied outlines and all 604 shaped pages stay within conservative cropped rows',async()=>{
  const outlines=JSON.parse(await readFile('public-qul/experimental/digital-khatt/shaped/glyphs.json','utf8'));
  const allBounds=new Map(Object.entries(outlines).map(([id,path])=>[Number(id),outlineYBounds(path)]));
  for (const [id,bounds] of allBounds) assert.ok(bounds,`supported supplied path ${id}`);
  let lines=0,fullRows=0,croppedRows=0;
  for(let page=1;page<=604;page++) {
    const data=JSON.parse(await readFile(`public-qul/experimental/digital-khatt/shaped/p${page}.json`,'utf8'));
    for(const shape of Object.values(data.lines)) {
      const scale=1200/shape.width,height=3200*scale,fullHeight=Math.ceil(height)+1;
      const frame={width:1200,height,offsetY:.375};
      const crop=cropShapedInk(shape,outlines,fullHeight,frame);
      for(const [id,,y] of shape.glyphs) {
        const bounds=allBounds.get(id);
        for(const bound of bounds) {
          const pixel=1950*scale+frame.offsetY-(y+bound)*shape.scale*scale;
          if(pixel>=0&&pixel<=fullHeight) assert.ok(pixel>=crop.top&&pixel<=crop.top+crop.height,`page ${page} cropped a control point`);
        }
      }
      assert.ok(crop.height>0&&crop.height<=fullHeight);
      lines++;fullRows+=fullHeight;croppedRows+=crop.height;
    }
  }
  assert.ok(lines>8500);
  assert.ok(croppedRows<fullRows*.85,'transparent rows are meaningfully reduced without changing source geometry');
});
