import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, lstatSync, readFileSync, readdirSync, realpathSync, writeFileSync } from 'node:fs';
import { join, resolve, sep } from 'node:path';

export const WEB_SOURCE_RECORD = 'WEB-BUILD-SOURCE.json';
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const normalized = path => process.platform === 'win32' ? resolve(path).toLowerCase() : resolve(path);

function sameDirectory(left, right) {
  try {
    // Git resolves junctions and Windows short-name paths in --show-toplevel.
    // Compare filesystem identities rather than rejecting an alias of this
    // checkout. Keep containment checks below lexical and symlink-rejecting.
    return normalized(realpathSync.native(left)) === normalized(realpathSync.native(right));
  } catch { return false; }
}

function relativePath(root, relative) {
  if (typeof relative !== 'string' || !relative || relative.includes('\\')
    || relative.split('/').some(part => !part || part === '.' || part === '..')) throw new Error('Unsafe source inventory path.');
  const absolute = resolve(root, relative);
  if (!normalized(absolute).startsWith(`${normalized(root)}${sep}`)) throw new Error('Source inventory path is outside its root.');
  return absolute;
}

function filesFingerprint(root, paths) {
  const hash = createHash('sha256');
  const unique = [...new Set(paths)].sort();
  for (const path of unique) {
    const absolute = relativePath(root, path);
    let content = 'missing-or-not-a-file';
    if (existsSync(absolute)) {
      const stat = lstatSync(absolute);
      if (stat.isSymbolicLink()) throw new Error(`Source identity cannot follow a symbolic link: ${path}`);
      if (stat.isFile()) content = digest(readFileSync(absolute));
    }
    hash.update(path).update('\0').update(content).update('\0');
  }
  return { treeContentSha256: hash.digest('hex'), fileCount: unique.length };
}

/** A content identity, not a digital signature or compiler reproducibility claim. */
export function sourceIdentity(sourceRoot) {
  const root = resolve(sourceRoot);
  const git = args => spawnSync('git', args, { cwd: root, encoding: 'utf8', windowsHide: true, maxBuffer: 16 * 1024 * 1024 });
  const top = git(['rev-parse', '--show-toplevel']);
  // An exported folder can live underneath a different checkout. Do not
  // accidentally attribute that folder to the enclosing repository's commit.
  if (!top.error && top.status === 0 && sameDirectory(top.stdout.trim(), root)) {
    const commit = git(['rev-parse', 'HEAD']);
    const status = git(['status', '--porcelain=v1', '--untracked-files=all']);
    const paths = git(['ls-files', '--cached', '--others', '--exclude-standard', '-z']);
    if (commit.error || commit.status !== 0 || status.error || status.status !== 0 || paths.error || paths.status !== 0) {
      throw new Error('Unable to inspect the Git working tree for build source identity.');
    }
    return { available: true, kind: 'git-working-tree', commit: commit.stdout.trim(), dirty: Boolean(status.stdout.trim()),
      statusSha256: digest(status.stdout), ...filesFingerprint(root, paths.stdout.split('\0').filter(Boolean)) };
  }
  if (existsSync(join(root, '.git'))) throw new Error('Unable to read this Git checkout. Source identity cannot be verified.');

  // Source ZIPs deliberately omit .git. Rehash the actual inventory bytes, not
  // the published hashes, so recipients may modify and rebuild their copy.
  // Walking the export also captures new, unlisted source files. Generated
  // directories and credentials are excluded; the two source DBs under work/
  // are included through the inventory rather than development caches.
  const inventory = join(root, 'SOURCE-FILES.sha256');
  if (!existsSync(inventory)) throw new Error('Build source needs an accessible Git checkout or SOURCE-FILES.sha256 source-export inventory.');
  const paths = readFileSync(inventory, 'utf8').split(/\r?\n/).filter(Boolean).map(line => {
    const match = line.match(/^[a-f0-9]{64}  (.+)$/i);
    if (!match) throw new Error('Invalid source-export inventory.');
    relativePath(root, match[1]); return match[1];
  });
  const excludedDirectories = new Set(['.git', 'node_modules', 'dist', 'outputs', 'work', '.npm-cache', '.codex', '.agents', '.aws', 'target', 'bin', 'obj']);
  const excludedFile = name => /^\.env(?:\.|$)/i.test(name) || /\.(?:user|suo|pfx|p12|key)$/i.test(name);
  function walk(directory, prefix = '') {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = `${prefix}${entry.name}`;
      if (entry.isSymbolicLink()) throw new Error(`Source identity cannot follow a symbolic link: ${path}`);
      if (entry.isDirectory()) { if (!excludedDirectories.has(entry.name)) walk(join(directory, entry.name), `${path}/`); }
      else if (entry.isFile() && !excludedFile(entry.name)) paths.push(path);
    }
  }
  walk(root);
  return { available: true, kind: 'source-export-inventory-and-walk', commit: null, dirty: null, statusSha256: null,
    scope: 'Actual inventory files plus export files outside generated/cache/credential directories. New source data under work/ requires adding it to SOURCE-FILES.sha256.',
    ...filesFingerprint(root, paths) };
}

export function assertSameBuildSource(before, after) {
  if (!before?.available || !after?.available || JSON.stringify(before) !== JSON.stringify(after)) {
    throw new Error('Source changed during compilation. Finish source edits and build again.');
  }
}

function webFingerprint(root) {
  const directory = join(root, 'dist');
  if (!existsSync(join(directory, 'index.html'))) throw new Error('The web build is missing. Run npm run build first.');
  const paths = [];
  function walk(folder, prefix = '') {
    for (const entry of readdirSync(folder, { withFileTypes: true })) {
      const path = `${prefix}${entry.name}`;
      if (entry.isSymbolicLink()) throw new Error('The web build must not contain symbolic links.');
      if (entry.isDirectory()) walk(join(folder, entry.name), `${path}/`);
      else if (entry.isFile() && path !== WEB_SOURCE_RECORD) paths.push(path);
    }
  }
  walk(directory);
  return filesFingerprint(directory, paths);
}

export function writeWebBuildSource(root, before, after = sourceIdentity(root)) {
  assertSameBuildSource(before, after);
  const record = { schemaVersion: 1, builtAt: new Date().toISOString(), source: after, webFiles: webFingerprint(root),
    meaning: 'Binds the web output bytes to this source snapshot. This is not a signature or a claim of byte-identical builds across machines.' };
  writeFileSync(join(root, 'dist', WEB_SOURCE_RECORD), `${JSON.stringify(record, null, 2)}\n`);
  return record;
}

export function verifyWebBuildSource(root, current = sourceIdentity(root)) {
  let record;
  try { record = JSON.parse(readFileSync(join(root, 'dist', WEB_SOURCE_RECORD), 'utf8')); }
  catch { throw new Error('Web build source record is missing or invalid. Run npm run build before building the Windows host.'); }
  if (record.schemaVersion !== 1 || !record.source?.available || !current?.available
    || !/^[a-f0-9]{64}$/.test(record.source.treeContentSha256 ?? '')
    || record.source.treeContentSha256 !== current.treeContentSha256) {
    throw new Error('The web build belongs to different or changed source. Run npm run build before building the Windows host.');
  }
  const actual = webFingerprint(root);
  if (actual.treeContentSha256 !== record.webFiles?.treeContentSha256 || actual.fileCount !== record.webFiles?.fileCount) {
    throw new Error('WebApp files changed after the web build was recorded. Run npm run build again.');
  }
  return record;
}
