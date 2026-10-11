import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';

const dist = resolve('dist');
const target = resolve(process.argv[2] || 'windows/QuranMushaf.Windows/bin/Release/net10.0-windows10.0.19041.0/win-x64/publish/WebApp');
const hash = path => createHash('sha256').update(readFileSync(path)).digest('hex');
let count = 0;
function verify(relative = '') {
  for (const item of readdirSync(join(dist, relative), {withFileTypes:true})) {
    const name = join(relative, item.name);
    if (item.isDirectory()) verify(name);
    else { assert.equal(hash(join(target,name)),hash(join(dist,name)),`Stale or missing packaged file: ${name}`); count++; }
  }
}
verify();
console.log(`Verified ${count} packaged web files against the tested build at ${target}.`);
