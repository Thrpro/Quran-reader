import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import {
  cpSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync,
  statSync, writeFileSync,
} from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const host = join(root, 'windows', 'QuranMushaf.Rust');
const toolchain = join(root, 'work', 'rust-toolchain');
const output = join(root, 'outputs', 'rust-windows');
const publish = join(output, 'publish');
const staging = join(output, 'publish-staging');
const packageJson = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));

if (process.platform !== 'win32' || process.arch !== 'x64') {
  throw new Error('Build the Windows x64 Rust host on an x64 Windows machine.');
}
for (const file of ['Cargo.toml', 'Cargo.lock']) {
  if (!existsSync(join(host, file))) throw new Error(`Missing Rust host ${file}.`);
}
const cargoPackage = readFileSync(join(host, 'Cargo.toml'), 'utf8').split('[package]')[1]?.split(/\r?\n\[/)[0];
const hostVersion = cargoPackage?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (hostVersion !== packageJson.version) {
  throw new Error('Rust Cargo.toml and package.json versions differ. Update both before publishing.');
}
if (!existsSync(join(root, 'dist', 'index.html'))) {
  throw new Error('The tested web build is missing. Run npm run build first.');
}

const where = spawnSync('where.exe', ['cargo.exe'], { encoding: 'utf8' });
const candidates = [
  process.env.CARGO,
  ...(where.stdout || '').trim().split(/\r?\n/).filter(Boolean),
  process.env.CARGO_HOME && join(process.env.CARGO_HOME, 'bin', 'cargo.exe'),
  join(toolchain, 'cargo', 'bin', 'cargo.exe'),
  process.env.USERPROFILE && join(process.env.USERPROFILE, '.cargo', 'bin', 'cargo.exe'),
].filter(Boolean);
const cargo = candidates.find(candidate => existsSync(candidate));
if (!cargo) {
  throw new Error('Rust is missing. Install the official minimal Rust MSVC toolchain or set CARGO to cargo.exe.');
}
const localCargo = resolve(cargo).toLowerCase().startsWith(`${resolve(toolchain).toLowerCase()}${sep}`);
const env = {
  ...process.env,
  ...(localCargo ? {
    CARGO_HOME: join(toolchain, 'cargo'),
    RUSTUP_HOME: join(toolchain, 'rustup'),
  } : {}),
  PATH: `${dirname(cargo)}${sep === '\\' ? ';' : ':'}${process.env.PATH || ''}`,
  CARGO_TARGET_DIR: join(host, 'target'),
};

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: 'inherit', ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed with exit code ${result.status}.`);
  return result;
}
const manifest = join(host, 'Cargo.toml');
const metadata = JSON.parse(run(cargo, ['metadata', '--locked', '--offline', '--format-version', '1',
  '--filter-platform', 'x86_64-pc-windows-msvc', '--manifest-path', manifest], {
  cwd: host, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 16 * 1024 * 1024,
}).stdout);
const nativeHostFeatures = metadata.resolve.nodes.find(node => node.id === metadata.resolve.root)?.features ?? [];
if (nativeHostFeatures.includes('qa')) throw new Error('The production host must not include the QA feature.');
const notices = rustNotices(metadata);
const rustSysroot = run(join(dirname(cargo), 'rustc.exe'), ['--print', 'sysroot'], {
  cwd: host, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'],
}).stdout.trim();
const rustLibraryCopyright = join(rustSysroot, 'share', 'doc', 'rust', 'COPYRIGHT-library.html');
if (!existsSync(rustLibraryCopyright)) throw new Error('The Rust standard-library copyright notice is missing from this toolchain.');
// Cargo discovers .cargo/config.toml from the working directory, including
// this host's static CRT setting; --manifest-path alone does not select it.
run(cargo, ['test', '--locked', '--manifest-path', manifest], { cwd: host });
run(cargo, ['build', '--release', '--locked', '--manifest-path', manifest], { cwd: host });
const binary = join(host, 'target', 'release', 'quran-mushaf-reader.exe');
if (!existsSync(binary)) throw new Error('Cargo did not produce the Windows host executable.');

// Only replace app-owned output directories. Never touch the shared reader profile.
function removeOutput(path) {
  const absolute = resolve(path);
  const prefix = `${resolve(output).toLowerCase()}${sep}`;
  if (!absolute.toLowerCase().startsWith(prefix)) throw new Error('Refusing to remove a directory outside the Rust output folder.');
  rmSync(absolute, { recursive: true, force: true });
}
mkdirSync(output, { recursive: true });
removeOutput(staging);
mkdirSync(staging);
cpSync(binary, join(staging, 'QuranMushaf.Windows.exe'));
cpSync(join(root, 'dist'), join(staging, 'WebApp'), { recursive: true });
mkdirSync(join(staging, 'Assets'));
cpSync(join(root, 'windows', 'QuranMushaf.Windows', 'Assets', 'app.ico'), join(staging, 'Assets', 'app.ico'));
for (const notice of ['LICENSE', 'THIRD-PARTY-NOTICES.md']) cpSync(join(root, notice), join(staging, notice));
writeFileSync(join(staging, 'THIRD-PARTY-RUST.txt'), notices.text);
cpSync(rustLibraryCopyright, join(staging, 'THIRD-PARTY-RUST-LIBRARY.html'));
console.log(`Included full license notices for ${notices.packages} Rust dependencies, the Rust standard library, and the Microsoft WebView2 loader.`);
run(process.execPath, [join(root, 'scripts', 'verify-packaged-web.mjs'), join(staging, 'WebApp')]);

function hash(path) { return createHash('sha256').update(readFileSync(path)).digest('hex'); }
function payloadFiles(directory, relative = '') {
  return readdirSync(join(directory, relative), { withFileTypes: true }).flatMap(entry => {
    const path = join(relative, entry.name);
    return entry.isDirectory() ? payloadFiles(directory, path) : [{
      path: path.split(sep).join('/'), bytes: statSync(join(directory, path)).size,
      sha256: hash(join(directory, path)),
    }];
  });
}
const files = payloadFiles(staging);
writeFileSync(join(staging, 'RUST-HOST-BUILD.json'), `${JSON.stringify({
  version: packageJson.version,
  nativeHost: 'rust',
  nativeHostFeatures,
  target: 'x86_64-pc-windows-msvc',
  builtAt: new Date().toISOString(),
  executableSha256: hash(join(staging, 'QuranMushaf.Windows.exe')),
  cargoLockSha256: hash(join(host, 'Cargo.lock')),
  rustDependencyNotices: notices.packages,
  payloadBytes: files.reduce((sum, file) => sum + file.bytes, 0),
  payloadBytesExcludesBuildManifest: true,
  files,
}, null, 2)}\n`);
removeOutput(publish);
renameSync(staging, publish);
console.log(`Published Rust Windows host at ${publish}`);
const publishedBytes = files.reduce((sum, file) => sum + file.bytes, 0) + statSync(join(publish, 'RUST-HOST-BUILD.json')).size;
console.log(`Payload: ${(publishedBytes / 1_000_000).toFixed(2)} MB, including ${(statSync(binary).size / 1_000_000).toFixed(2)} MB native executable.`);

function rustNotices(metadata) {
  const sourceFolder = join(root, 'windows', 'installer', 'licenses');
  const sources = JSON.parse(readFileSync(join(sourceFolder, 'SOURCES.json'), 'utf8'));
  const active = new Set(metadata.resolve.nodes.map(node => node.id));
  const packages = metadata.packages.filter(pkg => active.has(pkg.id) && !metadata.workspace_members.includes(pkg.id))
    .sort((left, right) => `${left.name}@${left.version}`.localeCompare(`${right.name}@${right.version}`));
  const sections = ['Quran Mushaf Reader — Rust third-party license notices',
    'This file includes runtime and build dependencies from the locked Cargo graph.',
    'Rust standard-library notices are included in THIRD-PARTY-RUST-LIBRARY.html.'];
  function vendoredText(source) {
    const path = join(sourceFolder, source.file);
    if (hash(path) !== source.sha256) throw new Error(`Changed vendored license: ${source.file}`);
    return readFileSync(path, 'utf8');
  }
  for (const pkg of packages) {
    const folder = dirname(pkg.manifest_path);
    const texts = new Map();
    function add(path, label) {
      const absolute = resolve(path);
      if (!absolute.toLowerCase().startsWith(`${resolve(folder).toLowerCase()}${sep}`)) {
        throw new Error(`License path is outside crate ${pkg.name}: ${label}`);
      }
      const text = readFileSync(absolute, 'utf8');
      if (!text.trim()) throw new Error(`Empty license for ${pkg.name}: ${label}`);
      texts.set(absolute, { label, text });
    }
    function collect(directory, depth = 0) {
      for (const entry of readdirSync(directory, { withFileTypes: true })) {
        if (!/^(?:licen[sc]es?|copying|notice|copyright)(?:[._-]|$)/i.test(entry.name)) continue;
        const path = join(directory, entry.name);
        if (entry.isFile()) add(path, path.slice(folder.length + 1));
        else if (entry.isDirectory() && depth < 2) {
          for (const child of readdirSync(path, { withFileTypes: true })) {
            if (child.isFile()) add(join(path, child.name), `${entry.name}/${child.name}`);
          }
          collect(path, depth + 1);
        }
      }
    }
    collect(folder);
    if (pkg.license_file) add(join(folder, pkg.license_file), pkg.license_file);
    if (!texts.size) {
      const expectedCommit = sources.webview2Bindings.packages[`${pkg.name}@${pkg.version}`];
      const vcs = existsSync(join(folder, '.cargo_vcs_info.json'))
        ? JSON.parse(readFileSync(join(folder, '.cargo_vcs_info.json'), 'utf8')) : null;
      if (!expectedCommit || vcs?.git?.sha1 !== expectedCommit || pkg.repository !== sources.webview2Bindings.repository) {
        throw new Error(`Missing full license text for ${pkg.name}@${pkg.version}; add its exact upstream notice before publishing.`);
      }
      texts.set('upstream', {
        label: `${sources.webview2Bindings.repository}/blob/${expectedCommit}/LICENSE`,
        text: vendoredText(sources.webview2Bindings),
      });
    }
    if (!pkg.license && !pkg.license_file) throw new Error(`Missing license metadata for ${pkg.name}@${pkg.version}.`);
    sections.push(`\n${'='.repeat(72)}\n${pkg.name} ${pkg.version}\nSPDX: ${pkg.license || 'Custom license — see license file'}\nRepository: ${pkg.repository || pkg.homepage || `https://crates.io/crates/${pkg.name}`}\n`);
    for (const { label, text } of texts.values()) sections.push(`--- ${label} ---\n${text}`);
  }
  const loader = packages.find(pkg => `${pkg.name}@${pkg.version}` === sources.webview2Sdk.crate);
  if (!loader || hash(join(dirname(loader.manifest_path), 'x64', 'WebView2LoaderStatic.lib')) !== sources.webview2Sdk.x64StaticLoaderSha256) {
    throw new Error('The WebView2 static loader differs from the SDK associated with the bundled Microsoft license. Update and verify its license first.');
  }
  sections.push(`\n${'='.repeat(72)}\nMicrosoft WebView2 SDK loader ${sources.webview2Sdk.version}\nSource: ${sources.webview2Sdk.url}\n${vendoredText(sources.webview2Sdk)}`);
  return { text: `${sections.join('\n')}\n`, packages: packages.length };
}
