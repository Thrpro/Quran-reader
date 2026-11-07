import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
// Development-only native-decoded fixture, never a shipped recitation.
export function longMp3(original, targetMiB = 22) {
  // Keep complete original MPEG-1 Layer III frames, strip stale ID3 metadata,
  // then prepend one metadata frame with truthful Info duration/byte/seek data.
  // This is repeated authentic Ikhlas audio, not a claimed Baqarah recording.
  const tagSize = original.subarray(0, 3).toString() === 'ID3'
    ? 10 + (original[6] << 21 | original[7] << 14 | original[8] << 7 | original[9]) : 0;
  let cursor = tagSize, frameCount = 0, rate = 0, firstLength = 0;
  while (cursor + 4 <= original.length) {
    const word = original.readUInt32BE(cursor);
    if ((word >>> 21) !== 0x7ff) break;
    assert.equal(word >>> 19 & 3, 3, 'fixture must be MPEG-1');
    assert.equal(word >>> 17 & 3, 1, 'fixture must be Layer III');
    const bitrate = [0,32,40,48,56,64,80,96,112,128,160,192,224,256,320][word >>> 12 & 15] * 1000;
    rate = [44100,48000,32000][word >>> 10 & 3];
    assert.ok(bitrate && rate);
    const length = Math.floor(144 * bitrate / rate) + (word >>> 9 & 1);
    if (cursor + length > original.length) break;
    firstLength ||= length; cursor += length; frameCount++;
  }
  assert.ok(frameCount > 100);
  const encoded = original.subarray(tagSize, cursor);
  const repetitions = Math.ceil(targetMiB * 1048576 / encoded.length);
  const info = Buffer.alloc(firstLength);
  original.copy(info, 0, tagSize, tagSize + 4);
  const mono = (info[3] >>> 6) === 3;
  const position = 4 + (mono ? 17 : 32);
  info.write('Info', position, 'ascii'); info.writeUInt32BE(7, position + 4);
  info.writeUInt32BE(frameCount * repetitions, position + 8);
  info.writeUInt32BE(info.length + encoded.length * repetitions, position + 12);
  for (let index = 0; index < 100; index++) info[position + 16 + index] = Math.floor(index * 256 / 100);
  const body = Buffer.concat([info, ...Array(repetitions).fill(encoded)]);
  const duration = frameCount * repetitions * 1152 / rate;
  const timingDurationMs = Math.floor((duration - .2) * 1000);
  const timings = Array.from({ length: 286 }, (_, index) => ({
    ayah: index + 1, start_time: Math.floor(index * timingDurationMs / 286),
    end_time: Math.floor((index + 1) * timingDurationMs / 286),
  }));
  return { body, timings, metadata: {
    originalFixture: 'tests/fixtures/yasser-continuous-112.mp3', originalSha256: digest(original),
    originalBytes: original.length, parsedAudioFrames: frameCount, repetitions, bytes: body.length,
    sha256: digest(body), expectedDurationSeconds: duration, sampleRate: rate,
    description: 'Losslessly repeated authentic Ikhlas MPEG frames with new Info counts/TOC. Synthetic 286-ayah timings span the media duration and exercise Baqarah page navigation; audio words do not correspond to those verses. No re-encoding, silence padding, or fake audio element.',
  } };
}
