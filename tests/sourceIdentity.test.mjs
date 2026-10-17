import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, rmdirSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, sep } from 'node:path';
import test from 'node:test';
import { sourceIdentity, writeWebBuildSource, verifyWebBuildSource, WEB_SOURCE_RECORD } from '../scripts/source-identity.mjs';

function fixture(run) {
  const root = mkdtempSync(join(tmpdir(), 'quran-source-identity-'));
  try {
    mkdirSync(join(root, 'src')); mkdirSync(join(root, 'dist'));
    writeFileSync(join(root, '.gitignore'), 'dist/\noutputs/\nnode_modules/\n');
    writeFileSync(join(root, 'src', 'app.ts'), 'export const version = 1;\n');
    writeFileSync(join(root, 'SOURCE-FILES.sha256'), `${'0'.repeat(64)}  src/app.ts\n`);
    writeFileSync(join(root, 'dist', 'index.html'), '<script src="app.js"></script>');
    writeFileSync(join(root, 'dist', 'app.js'), 'const compiled = 1;');
    return run(root);
  } finally {
    const absolute = resolve(root);
    assert.ok(absolute.startsWith(resolve(tmpdir()) + sep));
    rmSync(absolute, { recursive: true, force: true });
  }
}

test('source-export anchor allows modified source builds and rejects stale WebApp source', () => fixture(root => {
  const original = sourceIdentity(root);
  assert.equal(original.kind, 'source-export-inventory-and-walk');
  assert.equal(original.commit, null);
  writeWebBuildSource(root, original);
  assert.equal(verifyWebBuildSource(root).source.treeContentSha256, original.treeContentSha256);
  writeFileSync(join(root, 'src', 'app.ts'), 'export const version = 2;\n');
  assert.throws(() => verifyWebBuildSource(root), /web build belongs to different or changed source/);
  const modified = sourceIdentity(root);
  writeFileSync(join(root, 'dist', 'app.js'), 'const compiled = 2;');
  writeWebBuildSource(root, modified);
  assert.equal(verifyWebBuildSource(root).source.treeContentSha256, modified.treeContentSha256);
}));

test('new source files outside the export inventory cannot evade the anchor', () => fixture(root => {
  writeWebBuildSource(root, sourceIdentity(root));
  writeFileSync(join(root, 'src', 'new-module.ts'), 'export const added = true;');
  assert.throws(() => verifyWebBuildSource(root), /web build belongs to different or changed source/);
  writeWebBuildSource(root, sourceIdentity(root));
  mkdirSync(join(root, 'new-assets'));
  writeFileSync(join(root, 'new-assets', 'layout.json'), '{"added":true}');
  assert.throws(() => verifyWebBuildSource(root), /web build belongs to different or changed source/);
}));

test('web-output tampering and source changes during a build are refused', () => fixture(root => {
  const before = sourceIdentity(root);
  assert.throws(() => verifyWebBuildSource(root), /source record is missing or invalid/);
  writeWebBuildSource(root, before);
  const anchor = readFileSync(join(root, 'dist', WEB_SOURCE_RECORD));
  writeFileSync(join(root, 'dist', 'app.js'), 'const compiled = "different";');
  assert.throws(() => verifyWebBuildSource(root), /WebApp files changed after the web build was recorded/);
  writeFileSync(join(root, 'src', 'app.ts'), 'export const version = 3;');
  assert.throws(() => writeWebBuildSource(root, before), /Source changed during compilation/);
  assert.deepEqual(readFileSync(join(root, 'dist', WEB_SOURCE_RECORD)), anchor, 'failed compilation does not relabel the old output as new source');
}));

test('generated output does not create a source-fingerprint loop', () => fixture(root => {
  const before = sourceIdentity(root);
  writeWebBuildSource(root, before);
  mkdirSync(join(root, 'outputs')); writeFileSync(join(root, 'outputs', 'report.json'), '{}');
  mkdirSync(join(root, 'node_modules')); writeFileSync(join(root, 'node_modules', 'cache.txt'), 'cache');
  assert.deepEqual(sourceIdentity(root), before);
  assert.ok(verifyWebBuildSource(root));
}));

const gitAvailable = spawnSync('git', ['--version'], { encoding: 'utf8', windowsHide: true }).status === 0;
test('Git anchor covers tracked edits and new nonignored files while retaining commit identity', { skip: !gitAvailable }, () => fixture(root => {
  function git(args) {
    const result = spawnSync('git', args, { cwd: root, encoding: 'utf8', windowsHide: true });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  }
  git(['init', '--quiet']); git(['add', '.']);
  git(['-c', 'user.name=Build fixture', '-c', 'user.email=build-fixture@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'Fixture source']);
  const before = sourceIdentity(root);
  assert.equal(before.kind, 'git-working-tree'); assert.equal(before.dirty, false);
  assert.match(before.commit, /^[a-f0-9]{40,64}$/);
  writeWebBuildSource(root, before);
  assert.ok(verifyWebBuildSource(root));
  writeFileSync(join(root, 'src', 'new-git-module.ts'), 'export const trackedLater = true;');
  assert.equal(sourceIdentity(root).dirty, true);
  assert.throws(() => verifyWebBuildSource(root), /web build belongs to different or changed source/);
  assert.ok(existsSync(join(root, 'dist', WEB_SOURCE_RECORD)));
}));

test('Git anchor recognizes a checkout reached through a directory alias', { skip: !gitAvailable }, () => fixture(root => {
  function git(args) {
    const result = spawnSync('git', args, { cwd: root, encoding: 'utf8', windowsHide: true });
    assert.equal(result.status, 0, result.stderr);
  }
  git(['init', '--quiet']); git(['add', '.']);
  git(['-c', 'user.name=Build fixture', '-c', 'user.email=build-fixture@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'Fixture source']);
  const aliasDirectory = mkdtempSync(join(tmpdir(), 'quran-source-alias-'));
  const alias = join(aliasDirectory, 'checkout');
  let linked = false;
  try {
    symlinkSync(root, alias, process.platform === 'win32' ? 'junction' : 'dir'); linked = true;
    const direct = sourceIdentity(root), aliased = sourceIdentity(alias);
    assert.equal(aliased.kind, 'git-working-tree');
    assert.deepEqual(aliased, direct, 'an alias identifies the same source and commit');
    writeWebBuildSource(alias, aliased);
    assert.ok(verifyWebBuildSource(root));
    writeFileSync(join(root, 'src', 'new-alias-module.ts'), 'export const added = true;');
    assert.throws(() => verifyWebBuildSource(alias), /web build belongs to different or changed source/);
    assert.equal(sourceIdentity(alias).dirty, true);
  } finally {
    // Unlink the alias itself; never recursively delete its target.
    if (linked) unlinkSync(alias);
    rmdirSync(aliasDirectory);
  }
}));

test('an export inside another checkout cannot claim its enclosing Git identity', { skip: !gitAvailable }, () => fixture(root => {
  const init = spawnSync('git', ['init', '--quiet'], { cwd: root, encoding: 'utf8', windowsHide: true });
  assert.equal(init.status, 0, init.stderr);
  const exported = join(root, 'source-export');
  mkdirSync(exported);
  writeFileSync(join(exported, 'SOURCE-FILES.sha256'), `${'0'.repeat(64)}  app.ts\n`);
  writeFileSync(join(exported, 'app.ts'), 'export const source = true;');
  const identity = sourceIdentity(exported);
  assert.equal(identity.kind, 'source-export-inventory-and-walk');
  assert.equal(identity.commit, null);
}));
