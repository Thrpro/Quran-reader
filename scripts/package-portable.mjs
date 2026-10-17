import { createHash } from 'node:crypto';
import { closeSync, existsSync, mkdirSync, openSync, readFileSync, readdirSync, statSync, unlinkSync, writeFileSync, writeSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateRawSync, inflateRawSync } from 'node:zlib';

const crcTable = Uint32Array.from({ length: 256 }, (_, initial) => {
  let value = initial;
  for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ ((value & 1) ? 0xedb88320 : 0);
  return value >>> 0;
});
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const version = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version;
if (!/^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(version)) throw new Error('Invalid app version.');
const options = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index];
  if (!['--payload', '--output'].includes(key) || !process.argv[index + 1] || options.has(key)) {
    throw new Error('Usage: node scripts/package-portable.mjs [--payload folder] [--output new-file.zip]');
  }
  options.set(key, process.argv[index + 1]);
}
const payload = resolve(root, options.get('--payload') ?? 'outputs/rust-windows/publish');
const output = resolve(root, options.get('--output') ?? `outputs/releases/v${version}/QuranMushafReader-Portable-x64-${version}.zip`);
if (!output.toLowerCase().endsWith('.zip')) throw new Error('The output must be a ZIP file.');
if (existsSync(output) || existsSync(`${output}.sha256.txt`)) throw new Error('Artifact already exists. Preserve it; choose a new --output path.');
const manifestBytes = readFileSync(join(payload, 'RUST-HOST-BUILD.json'));
const manifest = JSON.parse(manifestBytes);
if (manifest.version !== version || manifest.nativeHost !== 'rust' || manifest.target !== 'x86_64-pc-windows-msvc'
  || !Array.isArray(manifest.files) || manifest.nativeHostFeatures?.includes('qa')) throw new Error('Unexpected build version, target, or features.');
if (manifest.source?.changedDuringBuild) throw new Error('Source changed during compilation; build again before packaging.');
if (process.env.GITHUB_ACTIONS === 'true' && (!manifest.source?.available || manifest.source.dirty || !manifest.source.commit)) {
  throw new Error('CI portable artifacts require a recorded, clean source commit.');
}
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
function safePath(relative) {
  if (typeof relative !== 'string' || !relative || relative.includes('\\') || relative.split('/').some(part => !part || part === '.' || part === '..')) {
    throw new Error('Unsafe payload path in build manifest.');
  }
  const absolute = resolve(payload, relative);
  if (!absolute.toLowerCase().startsWith(`${payload.toLowerCase()}${sep}`)) throw new Error('Payload entry is outside the app directory.');
  return absolute;
}
function listFiles(directory, prefix = '') {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    if (entry.isSymbolicLink()) throw new Error('Portable payload must not contain symbolic links.');
    const relative = `${prefix}${entry.name}`;
    return entry.isDirectory() ? listFiles(join(directory, entry.name), `${relative}/`) : [relative];
  });
}
const expected = [...manifest.files.map(file => file.path), 'RUST-HOST-BUILD.json'].sort();
if (new Set(expected).size !== expected.length || JSON.stringify(expected) !== JSON.stringify(listFiles(payload).sort())) {
  throw new Error('Payload files differ from the verified build manifest.');
}
for (const file of manifest.files) {
  const bytes = readFileSync(safePath(file.path));
  if (bytes.length !== file.bytes || sha(bytes) !== file.sha256) throw new Error(`Build payload changed: ${file.path}`);
}
const prefix = `QuranMushafReader-Portable-${version}/`;
const readme = Buffer.from(`Quran Mushaf Reader ${version} — Windows x64 portable app\r\n\r\nExtract the entire ZIP, then run QuranMushaf.Windows.exe.\r\nKeep Assets and WebApp beside the executable. No installer is needed.\r\nMicrosoft WebView2 Runtime must be installed on the PC.\r\nSettings and downloads use the existing Windows user profile.\r\nRUST-HOST-BUILD.json records source identity, tool versions, and payload hashes.\r\nThis archive is a compiled build; creating it does not publish a GitHub release.\r\n`, 'utf8');
const entries = [...manifest.files.map(file => ({ ...file, absolute: safePath(file.path) })),
  { path: 'RUST-HOST-BUILD.json', bytes: manifestBytes.length, sha256: sha(manifestBytes), content: manifestBytes },
  { path: 'START-HERE.txt', bytes: readme.length, sha256: sha(readme), content: readme }];
if (entries.length > 65535) throw new Error('ZIP64 archives are not supported by this packager.');
mkdirSync(dirname(output), { recursive: true });
let created = false;
try {
  const fd = openSync(output, 'wx');
  created = true;
  try {
    let offset = 0;
    const central = [];
    const stamp = new Date(manifest.builtAt);
    if (!Number.isFinite(stamp.getTime()) || stamp.getUTCFullYear() < 1980 || stamp.getUTCFullYear() > 2107) throw new Error('Invalid build timestamp.');
    const date = ((stamp.getUTCFullYear() - 1980) << 9) | ((stamp.getUTCMonth() + 1) << 5) | stamp.getUTCDate();
    const time = (stamp.getUTCHours() << 11) | (stamp.getUTCMinutes() << 5) | (stamp.getUTCSeconds() >> 1);
    const append = bytes => { writeSync(fd, bytes); offset += bytes.length; if (offset > 0xffffffff) throw new Error('ZIP64 required.'); };
    for (const entry of entries) {
      const bytes = entry.content ?? readFileSync(entry.absolute);
      // Recheck immediately before writing to catch changes after initial validation.
      if (bytes.length !== entry.bytes || sha(bytes) !== entry.sha256) throw new Error(`Payload changed while packaging: ${entry.path}`);
      const name = Buffer.from(prefix + entry.path, 'utf8');
      if (name.length > 65535) throw new Error('ZIP entry name is too long.');
      const compressed = deflateRawSync(bytes, { level: 9 });
      const crc = crc32(bytes);
      const local = Buffer.alloc(30);
      local.writeUInt32LE(0x04034b50); local.writeUInt16LE(20, 4); local.writeUInt16LE(0x800, 6);
      local.writeUInt16LE(8, 8); local.writeUInt16LE(time, 10); local.writeUInt16LE(date, 12);
      local.writeUInt32LE(crc, 14); local.writeUInt32LE(compressed.length, 18); local.writeUInt32LE(bytes.length, 22); local.writeUInt16LE(name.length, 26);
      const record = Buffer.alloc(46);
      record.writeUInt32LE(0x02014b50); record.writeUInt16LE(20, 4); record.writeUInt16LE(20, 6);
      record.writeUInt16LE(0x800, 8); record.writeUInt16LE(8, 10); record.writeUInt16LE(time, 12); record.writeUInt16LE(date, 14);
      record.writeUInt32LE(crc, 16); record.writeUInt32LE(compressed.length, 20); record.writeUInt32LE(bytes.length, 24);
      record.writeUInt16LE(name.length, 28); record.writeUInt32LE(offset, 42);
      central.push(record, name);
      append(local); append(name); append(compressed);
    }
    const directoryOffset = offset;
    for (const record of central) append(record);
    const end = Buffer.alloc(22);
    end.writeUInt32LE(0x06054b50); end.writeUInt16LE(entries.length, 8); end.writeUInt16LE(entries.length, 10);
    end.writeUInt32LE(offset - directoryOffset, 12); end.writeUInt32LE(directoryOffset, 16); append(end);
  } finally { closeSync(fd); }

  // Read the completed archive independently and verify each decompressed entry.
  const archive = readFileSync(output);
  let cursor = 0;
  for (const entry of entries) {
    if (archive.readUInt32LE(cursor) !== 0x04034b50) throw new Error('Invalid ZIP local header.');
    const compressedSize = archive.readUInt32LE(cursor + 18);
    const nameLength = archive.readUInt16LE(cursor + 26);
    const extraLength = archive.readUInt16LE(cursor + 28);
    const name = archive.subarray(cursor + 30, cursor + 30 + nameLength).toString('utf8');
    const start = cursor + 30 + nameLength + extraLength;
    const bytes = inflateRawSync(archive.subarray(start, start + compressedSize));
    if (name !== prefix + entry.path || bytes.length !== entry.bytes || sha(bytes) !== entry.sha256 || crc32(bytes) !== archive.readUInt32LE(cursor + 14)) {
      throw new Error(`Archive validation failed: ${entry.path}`);
    }
    cursor = start + compressedSize;
  }
  if (archive.readUInt32LE(cursor) !== 0x02014b50 || archive.readUInt32LE(archive.length - 22) !== 0x06054b50
    || archive.readUInt16LE(archive.length - 12) !== entries.length || archive.readUInt32LE(archive.length - 6) !== cursor) {
    throw new Error('ZIP directory validation failed.');
  }
  const hash = sha(archive);
  writeFileSync(`${output}.sha256.txt`, `${hash}  ${output.split(sep).at(-1)}\n`, { flag: 'wx', encoding: 'ascii' });
  console.log(`Verified ${entries.length} archived files (${(statSync(output).size / 1_000_000).toFixed(2)} MB).\nZIP: ${output}\nSHA256: ${hash}`);
} catch (error) {
  // Only remove the new incomplete file created by this invocation.
  if (created && existsSync(output)) unlinkSync(output);
  throw error;
}

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = (crc >>> 8) ^ crcTable[(crc ^ byte) & 0xff];
  return (crc ^ 0xffffffff) >>> 0;
}
