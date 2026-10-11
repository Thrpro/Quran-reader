import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const lockPath = join(root, 'windows', 'QuranMushaf.Rust', 'Cargo.lock');
const output = join(root, 'outputs', 'dependency-check', 'rust-osv.json');
const hash = value => createHash('sha256').update(value).digest('hex');
const report = {
  checkedAt: new Date().toISOString(),
  source: 'https://api.osv.dev/v1/querybatch',
  cargoLockSha256: null,
  scope: 'All registry and Git dependencies in Cargo.lock, including build dependencies.',
  status: 'checking',
  packages: [],
  responses: [],
};
mkdirSync(dirname(output), { recursive: true });
function save() { writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`); }
save();

try {
  const lock = readFileSync(lockPath, 'utf8');
  report.cargoLockSha256 = hash(lock);
  const packages = lock.split(/^\[\[package\]\]\s*$/m).slice(1).map(block => {
    const field = name => block.match(new RegExp(`^${name} = "([^"\\r\\n]+)"`, 'm'))?.[1];
    return { name: field('name'), version: field('version'), source: field('source') };
  }).filter(pkg => pkg.source);
  if (!packages.length) throw new Error('Cargo.lock contains no dependency packages to verify.');
  let pending = packages.map(pkg => {
    if (!pkg.name || !pkg.version) throw new Error('Cargo.lock contains an incomplete package record.');
    let query;
    if (pkg.source === 'registry+https://github.com/rust-lang/crates.io-index' || pkg.source === 'sparse+https://index.crates.io/') {
      query = { package: { ecosystem: 'crates.io', name: pkg.name }, version: pkg.version };
    } else if (pkg.source.startsWith('git+') && /#[a-f0-9]{40}$/i.test(pkg.source)) {
      query = { commit: pkg.source.split('#').at(-1) };
    } else {
      throw new Error(`Unsupported registry for ${pkg.name}; this dependency could not be verified.`);
    }
    const entry = { name: pkg.name, version: pkg.version, vulnerabilities: [] };
    report.packages.push(entry);
    return { query, entry };
  });
  for (let page = 0; pending.length; page++) {
    if (page >= 20) throw new Error('OSV returned too many pages; the dependency scan is incomplete.');
    const response = await fetch(report.source, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ queries: pending.map(item => item.query) }),
      signal: AbortSignal.timeout(30_000),
    });
    if (!response.ok) throw new Error(`OSV returned HTTP ${response.status}; dependency safety could not be verified.`);
    const result = await response.json();
    if (!Array.isArray(result.results) || result.results.length !== pending.length) {
      throw new Error('OSV returned an incomplete batch response.');
    }
    report.responses.push(result);
    const next = [];
    result.results.forEach((item, index) => {
      if (!item || item.error || !Array.isArray(item.vulns ?? [])) throw new Error('OSV could not verify one of the dependency packages.');
      for (const vulnerability of item.vulns ?? []) {
        if (typeof vulnerability.id !== 'string') throw new Error('OSV returned an invalid advisory.');
        if (!pending[index].entry.vulnerabilities.includes(vulnerability.id)) {
          pending[index].entry.vulnerabilities.push(vulnerability.id);
        }
      }
      if (item.next_page_token) {
        next.push({ ...pending[index], query: { ...pending[index].query, page_token: item.next_page_token } });
      }
    });
    pending = next;
  }
  if (hash(readFileSync(lockPath, 'utf8')) !== report.cargoLockSha256) {
    throw new Error('Cargo.lock changed during the scan; verify the current lock file again.');
  }
  const findings = report.packages.filter(pkg => pkg.vulnerabilities.length);
  report.status = findings.length ? 'vulnerabilities_found' : 'passed';
  report.packageCount = report.packages.length;
  report.findingCount = findings.reduce((sum, pkg) => sum + pkg.vulnerabilities.length, 0);
  save();
  if (findings.length) {
    for (const pkg of findings) console.error(`${pkg.name} ${pkg.version}: ${pkg.vulnerabilities.join(', ')}`);
    console.error(`Rust dependency scan reported advisories. See ${output}.`);
    process.exitCode = 1;
  } else {
    console.log(`OSV found no reported vulnerabilities in ${report.packageCount} locked Rust dependencies. Report: ${output}`);
  }
} catch (error) {
  report.status = 'could_not_verify';
  report.error = error instanceof Error ? error.message : String(error);
  save();
  console.error(`Rust dependency scan could not verify the dependencies: ${report.error}`);
  console.error(`Report: ${output}`);
  process.exitCode = 1;
}
