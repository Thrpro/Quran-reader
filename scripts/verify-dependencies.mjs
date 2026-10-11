import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = join(root, 'outputs', 'dependency-check');
const legacy = process.argv.includes('--legacy');
mkdirSync(output, {recursive:true});
const npm = process.platform === 'win32'
  ? spawnSync('cmd.exe', ['/d','/s','/c','npm audit --json'], {cwd:root, encoding:'utf8'})
  : spawnSync('npm', ['audit','--json'], {cwd:root, encoding:'utf8'});
const stdout = npm.stdout || '';
writeFileSync(join(output, 'npm.json'), stdout || npm.stderr || npm.error?.message || '');
if (npm.error || !stdout.trim()) throw new Error('npm vulnerability scan could not verify dependencies. See outputs/dependency-check/npm.json.');
// Some npm versions append a notice after the JSON response.
const jsonEnd = stdout.lastIndexOf('}');
const report = JSON.parse(stdout.slice(0, jsonEnd + 1));
if (npm.error || npm.status !== 0 || report.error || !report.metadata || report.metadata.vulnerabilities.total) {
  throw new Error('Dependency audit failed or reported vulnerable npm packages. See outputs/dependency-check/npm.json.');
}
if (legacy) {
  const nuget = spawnSync('dotnet', ['list','windows/QuranMushaf.Windows/QuranMushaf.Windows.csproj',
    'package','--vulnerable','--include-transitive','--format','json'], {cwd:root, encoding:'utf8'});
  writeFileSync(join(output, 'nuget.json'), nuget.stdout || nuget.stderr || '');
  if (nuget.error || nuget.status !== 0) throw new Error('NuGet vulnerability scan failed.');
  const packages = JSON.parse(nuget.stdout).projects.flatMap(project => project.frameworks ?? [])
    .flatMap(framework => [...(framework.topLevelPackages ?? []), ...(framework.transitivePackages ?? [])]);
  if (packages.some(pkg => pkg.vulnerabilities?.length)) throw new Error('NuGet reported vulnerable packages.');
  console.log('npm and legacy NuGet direct/transitive vulnerability scans passed.');
} else {
  const rust = spawnSync(process.execPath, [join(root, 'scripts', 'verify-rust-dependencies.mjs')], {cwd:root, stdio:'inherit'});
  if (rust.error || rust.status !== 0) throw new Error('Rust dependency verification failed. See outputs/dependency-check/rust-osv.json.');
  console.log('npm and locked Rust direct/transitive vulnerability scans passed.');
}
