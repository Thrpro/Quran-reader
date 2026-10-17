import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const version = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version;
const packager = join(root, 'scripts', 'package-portable.mjs');

test('portable packaging verifies payloads, preserves existing files, and rejects changed inputs', () => {
  const temporary = mkdtempSync(join(tmpdir(), 'quran-package-check-'));
  try {
    const payload = join(temporary, 'payload');
    mkdirSync(join(payload, 'WebApp'), { recursive: true });
    const content = Buffer.from('Authentic fixture bytes \u0627\u0644\u0639\u0631\u0628\u064a\u0629\n');
    writeFileSync(join(payload, 'WebApp', 'fixture.txt'), content);
    const manifest = {
      version, nativeHost: 'rust', target: 'x86_64-pc-windows-msvc', nativeHostFeatures: [], builtAt: '2026-10-17T12:34:56.000Z',
      source: { available: true, commit: 'a'.repeat(40), dirty: false, changedDuringBuild: false },
      files: [{ path: 'WebApp/fixture.txt', bytes: content.length, sha256: createHash('sha256').update(content).digest('hex') }],
    };
    const manifestPath = join(payload, 'RUST-HOST-BUILD.json');
    writeFileSync(manifestPath, JSON.stringify(manifest));
    const output = join(temporary, 'portable.zip');
    const args = [packager, '--payload', payload, '--output', output];
    const built = spawnSync(process.execPath, args, { cwd: root, encoding: 'utf8' });
    assert.equal(built.status, 0, built.stderr);
    const archive = readFileSync(output);
    const archiveHash = createHash('sha256').update(archive).digest('hex');
    assert.match(readFileSync(`${output}.sha256.txt`, 'utf8'), new RegExp(`^${archiveHash}  portable\\.zip`));
    const repeated = spawnSync(process.execPath, args, { cwd: root, encoding: 'utf8' });
    assert.notEqual(repeated.status, 0);
    assert.match(repeated.stderr, /Artifact already exists/);
    assert.deepEqual(readFileSync(output), archive);
    if (process.platform === 'win32') {
      // A separate ZIP reader checks the directory records and extracted bytes.
      const command = "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.IO.Compression.FileSystem; "
        + "$zip=[System.IO.Compression.ZipFile]::OpenRead($env:PORTABLE_TEST_ARCHIVE); "
        + "try { $entry=$zip.GetEntry($env:PORTABLE_TEST_ENTRY); if ($null -eq $entry -or $zip.Entries.Count -ne 3) { throw 'Unexpected ZIP contents' }; "
        + "$stream=$entry.Open(); $hash=[System.Security.Cryptography.SHA256]::Create(); "
        + "try { [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-','').ToLowerInvariant() } finally { $hash.Dispose(); $stream.Dispose() } } finally { $zip.Dispose() }";
      const extractedHash = execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', command], {
        encoding: 'utf8', windowsHide: true,
        env: { ...process.env, PORTABLE_TEST_ARCHIVE: output, PORTABLE_TEST_ENTRY: `QuranMushafReader-Portable-${version}/WebApp/fixture.txt` },
      }).trim();
      assert.equal(extractedHash, manifest.files[0].sha256);
    }
    writeFileSync(join(payload, 'WebApp', 'fixture.txt'), 'Changed fixture');
    const changed = spawnSync(process.execPath, [packager, '--payload', payload, '--output', join(temporary, 'changed.zip')], { cwd: root, encoding: 'utf8' });
    assert.notEqual(changed.status, 0);
    assert.match(changed.stderr, /Build payload changed/);
    manifest.source.changedDuringBuild = true;
    writeFileSync(manifestPath, JSON.stringify(manifest));
    const changedSource = spawnSync(process.execPath, [packager, '--payload', payload, '--output', join(temporary, 'changed-source.zip')], { cwd: root, encoding: 'utf8' });
    assert.notEqual(changedSource.status, 0);
    assert.match(changedSource.stderr, /Source changed during compilation/);
  } finally {
    const absolute = resolve(temporary);
    assert.ok(absolute.startsWith(resolve(tmpdir()) + '\\') || absolute.startsWith(resolve(tmpdir()) + '/'));
    rmSync(absolute, { recursive: true, force: true });
  }
});
