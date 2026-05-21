// Generate a 16-bit PCM WAV fixture by invoking `modem tx-wav` and
// post-converting from the f32 format `hound` writes. 16-bit PCM is the
// format Chromium's `--use-file-for-fake-audio-capture` accepts reliably.
//
// Used at Playwright globalSetup time; safe to call repeatedly (mtime cache).

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "../../..");
const FIXTURES = __dirname;

/** Render a fixture; returns the absolute path to the s16 WAV. */
export function renderFixture(profile, message) {
  if (profile !== "audible" && profile !== "ultrasonic") {
    throw new Error(`unknown profile: ${profile}`);
  }
  mkdirSync(FIXTURES, { recursive: true });
  const inputTxt = path.join(FIXTURES, `${profile}.input.txt`);
  const wavF32 = path.join(FIXTURES, `${profile}.f32.wav`);
  const wavS16 = path.join(FIXTURES, `${profile}.s16.wav`);

  // Skip if the input hasn't changed and the s16 WAV already exists.
  const inputUnchanged =
    existsSync(inputTxt) && readFileSync(inputTxt, "utf8") === message;
  if (inputUnchanged && existsSync(wavS16) && existsSync(wavF32)
      && statSync(wavS16).mtimeMs >= statSync(wavF32).mtimeMs) {
    return wavS16;
  }

  writeFileSync(inputTxt, message, "utf8");

  // Shell out to the CLI. `--release` so subsequent runs reuse the cache.
  const res = spawnSync(
    "cargo",
    [
      "run",
      "--release",
      "--quiet",
      "-p",
      "modem-cli",
      "--",
      "tx-wav",
      "--profile",
      profile,
      inputTxt,
      wavF32,
    ],
    { cwd: REPO_ROOT, stdio: ["ignore", "inherit", "inherit"] },
  );
  if (res.status !== 0) {
    throw new Error(`cargo tx-wav exited ${res.status}`);
  }

  convertF32ToS16(wavF32, wavS16);
  return wavS16;
}

/** Read a 32-bit float WAV and write a 16-bit PCM WAV with the same sample rate. */
function convertF32ToS16(src, dst) {
  const buf = readFileSync(src);
  // Minimal WAV parser. The file `hound` writes has the form:
  //   RIFF <size> WAVE fmt  <fmtsize> <fmtdata...> [optional chunks] data <size> <samples...>
  // fmt for IEEE float: audioFormat=3, bitsPerSample=32.
  if (buf.toString("ascii", 0, 4) !== "RIFF" || buf.toString("ascii", 8, 12) !== "WAVE") {
    throw new Error("not a RIFF/WAVE file: " + src);
  }
  let pos = 12;
  let sampleRate = 0;
  let channels = 0;
  let dataOff = -1;
  let dataLen = 0;
  while (pos + 8 <= buf.length) {
    const id = buf.toString("ascii", pos, pos + 4);
    const size = buf.readUInt32LE(pos + 4);
    if (id === "fmt ") {
      const audioFormat = buf.readUInt16LE(pos + 8);
      channels = buf.readUInt16LE(pos + 10);
      sampleRate = buf.readUInt32LE(pos + 12);
      const bits = buf.readUInt16LE(pos + 22);
      // hound may write WAVE_FORMAT_EXTENSIBLE (0xFFFE) with an IEEE-float
      // SubFormat GUID instead of plain audioFormat=3. Accept both.
      const isPlainFloat = audioFormat === 3 && bits === 32;
      // EXTENSIBLE: SubFormat GUID starts at fmt body offset +24
      // (after audioFormat/channels/sampleRate/byteRate/blockAlign/
      // bitsPerSample/cbSize/validBits/channelMask). The first u16 of the
      // GUID encodes the sub-format code (LE); 3 = IEEE float.
      const isExtFloat =
        audioFormat === 0xfffe &&
        bits === 32 &&
        size >= 40 &&
        buf.readUInt16LE(pos + 8 + 24) === 3;
      if (!isPlainFloat && !isExtFloat) {
        throw new Error(`expected IEEE float WAV (fmt 3 or extensible/float, 32-bit); got fmt ${audioFormat} bits ${bits}`);
      }
    } else if (id === "data") {
      dataOff = pos + 8;
      dataLen = size;
      break;
    }
    pos += 8 + size + (size % 2);
  }
  if (dataOff < 0) throw new Error("no data chunk in " + src);
  if (channels !== 1) throw new Error("expected mono, got " + channels + " channels");

  const sampleCount = Math.floor(dataLen / 4);
  const out = Buffer.alloc(44 + sampleCount * 2);
  // RIFF header
  out.write("RIFF", 0, "ascii");
  out.writeUInt32LE(36 + sampleCount * 2, 4);
  out.write("WAVE", 8, "ascii");
  // fmt chunk (16 bytes, PCM)
  out.write("fmt ", 12, "ascii");
  out.writeUInt32LE(16, 16);
  out.writeUInt16LE(1, 20); // PCM
  out.writeUInt16LE(1, 22); // mono
  out.writeUInt32LE(sampleRate, 24);
  out.writeUInt32LE(sampleRate * 2, 28); // byte rate
  out.writeUInt16LE(2, 32); // block align
  out.writeUInt16LE(16, 34); // bits per sample
  // data chunk
  out.write("data", 36, "ascii");
  out.writeUInt32LE(sampleCount * 2, 40);
  for (let i = 0; i < sampleCount; i++) {
    const f = buf.readFloatLE(dataOff + i * 4);
    let s = Math.round(Math.max(-1, Math.min(1, f)) * 32767);
    out.writeInt16LE(s, 44 + i * 2);
  }
  writeFileSync(dst, out);
}
