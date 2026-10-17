import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { sourceIdentity, writeWebBuildSource } from './source-identity.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
if (process.argv.length > 2) throw new Error('Usage: node scripts/build-web.mjs');
const before = sourceIdentity(root);
for (const [script, args] of [
  [join(root, 'node_modules', 'typescript', 'bin', 'tsc'), []],
  [join(root, 'node_modules', 'vite', 'bin', 'vite.js'), ['build', '--configLoader', 'native']],
  [join(root, 'scripts', 'prune-dist-assets.mjs'), []],
]) {
  if (!existsSync(script)) throw new Error('Build dependencies are missing. Run npm ci first.');
  const result = spawnSync(process.execPath, [script, ...args], { cwd: root, env: process.env, stdio: 'inherit', windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Web build step failed with exit code ${result.status}: ${script}`);
}
const record = writeWebBuildSource(root, before);
console.log(`Recorded ${record.webFiles.fileCount} web files against source ${record.source.treeContentSha256}.`);
