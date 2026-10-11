// MP3 providers sometimes return an HTML/JSON error with HTTP 200. Validate
// media independently of MIME type (legitimate servers use octet-stream).
export const MAX_AUDIO_PREFIX = 1024 * 1024;
export function isErrorContentType(contentType: string | null) {
  return /(?:text\/|json|xml|javascript)/i.test(contentType ?? "");
}
export function mp3Signature(bytes: Uint8Array): "valid" | "invalid" | "pending" {
  if (bytes.length < 10) return "pending";
  let offset = 0;
  if (bytes[0] === 0x49 && bytes[1] === 0x44 && bytes[2] === 0x33) {
    if (bytes[3] < 2 || bytes[3] > 4 || bytes.slice(6, 10).some(value => value > 127)) return "invalid";
    const tagSize = bytes[6] * 2097152 + bytes[7] * 16384 + bytes[8] * 128 + bytes[9];
    offset = 10 + tagSize + (bytes[3] === 4 && (bytes[5] & 16) ? 10 : 0);
    if (offset + 4 > MAX_AUDIO_PREFIX) return "invalid";
  }
  if (bytes.length < offset + 4) return "pending";
  const [a, b, c] = bytes.subarray(offset, offset + 3);
  // MPEG sync, valid version, Layer III, non-reserved bitrate/sample rate.
  return a === 255 && (b & 224) === 224 && (b & 24) !== 8 && (b & 6) === 2 &&
    (c & 240) !== 0 && (c & 240) !== 240 && (c & 12) !== 12 ? "valid" : "invalid";
}

/** Inspect at most one bounded prefix; never duplicate a full surah in memory. */
export async function validAudioResponse(response: Response) {
  if (isErrorContentType(response.headers.get("content-type")) || !response.body) return false;
  const reader = response.body.getReader();
  let prefix = new Uint8Array(0);
  try {
    while (prefix.length < MAX_AUDIO_PREFIX) {
      const { value, done } = await reader.read();
      if (done) return false;
      const next = new Uint8Array(Math.min(MAX_AUDIO_PREFIX, prefix.length + value.length));
      next.set(prefix); next.set(value.subarray(0, next.length - prefix.length), prefix.length); prefix = next;
      const signature = mp3Signature(prefix);
      if (signature !== "pending") return signature === "valid";
    }
    return false;
  } finally { void reader.cancel().catch(() => {}); reader.releaseLock(); }
}

/** Ayah clips are small enough to inspect completely before playback. A valid
 * first header alone also accepts a cut-off recording, which would skip its end. */
export function completeMp3(bytes: Uint8Array) {
  if (mp3Signature(bytes) !== "valid") return false;
  let offset = 0;
  if (bytes[0] === 73 && bytes[1] === 68 && bytes[2] === 51) {
    offset = 10 + bytes[6] * 2097152 + bytes[7] * 16384 + bytes[8] * 128 + bytes[9];
    if (bytes[3] === 4 && (bytes[5] & 16)) offset += 10;
  }
  let frames = 0, expectedFrames = 0;
  const rates = [44100, 48000, 32000];
  const mpeg1 = [0,32,40,48,56,64,80,96,112,128,160,192,224,256,320];
  const mpeg2 = [0,8,16,24,32,40,48,56,64,80,96,112,128,144,160];
  while (offset < bytes.length) {
    // ID3v1 footer and encoder zero padding contain no audio frames.
    if (bytes.length - offset === 128 && bytes[offset] === 84 && bytes[offset + 1] === 65 && bytes[offset + 2] === 71) break;
    if (bytes[offset] === 0 && bytes.subarray(offset).every(value => value === 0)) break;
    if (offset + 4 > bytes.length) return false;
    const a = bytes[offset], b = bytes[offset + 1], c = bytes[offset + 2];
    const version = (b >> 3) & 3, rateIndex = (c >> 2) & 3, bitrateIndex = c >> 4;
    if (a !== 255 || (b & 224) !== 224 || version === 1 || (b & 6) !== 2 || rateIndex === 3 || !bitrateIndex || bitrateIndex === 15) return false;
    const rate = rates[rateIndex] / (version === 3 ? 1 : version === 2 ? 2 : 4);
    const length = Math.floor((version === 3 ? 144000 : 72000) * (version === 3 ? mpeg1 : mpeg2)[bitrateIndex] / rate) + ((c >> 1) & 1);
    if (offset + length > bytes.length) return false;
    if (frames === 0) {
      const mono = (bytes[offset + 3] >> 6) === 3;
      const sideInfo = version === 3 ? (mono ? 17 : 32) : (mono ? 9 : 17);
      const xing = offset + 4 + ((b & 1) ? 0 : 2) + sideInfo;
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      if (xing + 12 <= offset + length && ["Xing", "Info"].includes(String.fromCharCode(...bytes.subarray(xing, xing + 4))) && (view.getUint32(xing + 4) & 1)) {
        expectedFrames = view.getUint32(xing + 8);
      }
    }
    frames++; offset += length;
  }
  // Some encoders exclude the Xing header frame from the declared count.
  return frames > 0 && frames >= expectedFrames;
}
