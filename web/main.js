import init, {
  WasmTransmitter,
  WasmReceiver,
  sampleRate as wasmSampleRate,
  toneFreqs,
  goertzel,
} from "./pkg/modem_wasm.js";

const TARGET_SR = 48000;
const DB_FLOOR = -80;
const DB_CEIL = 0;
const TONE_EMA = 0.4;

const els = {
  status: document.getElementById("status"),
  log: document.getElementById("log"),
  msg: document.getElementById("msg"),
  send: document.getElementById("send-btn"),
  listen: document.getElementById("listen-btn"),
  mic: document.getElementById("mic-status"),
  tones: document.getElementById("tones"),
  freqLabels: document.getElementById("freq-labels"),
};

const state = {
  ctx: null,
  workletReady: false,
  micNode: null,
  workletNode: null,
  rx: null,
  listening: false,
  tonesDb: new Float32Array(8).fill(DB_FLOOR),
  freqs: new Float32Array(8),
  // TX playback state for live tone-bar update
  tx: { active: false, samples: null, start: 0, total: 0 },
};

function getProfile() {
  const v = document.querySelector('input[name="profile"]:checked')?.value;
  return v || "audible";
}

function log(text, cls = "log-info") {
  const div = document.createElement("div");
  div.className = "log-line " + cls;
  div.textContent = text;
  els.log.appendChild(div);
  els.log.scrollTop = els.log.scrollHeight;
}

function refreshFreqLabels() {
  const freqs = toneFreqs(getProfile());
  state.freqs = Float32Array.from(freqs);
  els.freqLabels.innerHTML = "";
  for (const f of freqs) {
    const span = document.createElement("span");
    span.textContent = (f / 1000).toFixed(2) + "k";
    els.freqLabels.appendChild(span);
  }
}

function drawTones() {
  const c = els.tones;
  const ctx = c.getContext("2d");
  // Resize backing store to actual CSS pixels for crispness
  const rect = c.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  if (c.width !== Math.round(rect.width * dpr)) {
    c.width = Math.round(rect.width * dpr);
    c.height = Math.round(rect.height * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const W = rect.width;
  const H = rect.height;
  ctx.fillStyle = "#0d0f12";
  ctx.fillRect(0, 0, W, H);
  const n = state.tonesDb.length;
  const barW = W / (n * 2);
  for (let i = 0; i < n; i++) {
    const lvl = Math.min(1, Math.max(0, (state.tonesDb[i] - DB_FLOOR) / (DB_CEIL - DB_FLOOR)));
    const h = lvl * H;
    const x = (i * 2 + 0.5) * barW;
    ctx.fillStyle = `rgba(76, 175, 80, ${0.3 + 0.7 * lvl})`;
    ctx.fillRect(x, H - h, barW, h);
  }
}

function pushTones(samples) {
  // Run Goertzel for the active profile and EMA-smooth into state.tonesDb.
  const mags = goertzel(samples, getProfile());
  const norm = (samples.length * samples.length) || 1;
  for (let i = 0; i < mags.length; i++) {
    const v = mags[i] / norm;
    const db = v > 0 ? 10 * Math.log10(v) : DB_FLOOR;
    state.tonesDb[i] += TONE_EMA * (db - state.tonesDb[i]);
  }
}

let drawScheduled = false;
function scheduleDraw() {
  if (drawScheduled) return;
  drawScheduled = true;
  requestAnimationFrame(() => {
    drawScheduled = false;
    drawTones();
  });
}

async function ensureCtx() {
  if (state.ctx) return state.ctx;
  els.status.textContent = "starting audio…";
  state.ctx = new AudioContext({ sampleRate: TARGET_SR });
  if (state.ctx.sampleRate !== TARGET_SR) {
    log(
      `⚠ AudioContext returned ${state.ctx.sampleRate} Hz instead of ${TARGET_SR} Hz — ` +
        `decode may misalign on this device.`,
      "log-info",
    );
  }
  await state.ctx.audioWorklet.addModule("./worklet.js");
  state.workletReady = true;
  els.status.textContent = `audio ready @ ${state.ctx.sampleRate} Hz`;
  return state.ctx;
}

async function send() {
  const text = els.msg.value;
  if (!text) return;
  const profile = getProfile();
  await ensureCtx();
  await state.ctx.resume();

  const tx = new WasmTransmitter(profile);
  const samples = tx.encode(new TextEncoder().encode(text));

  const buf = state.ctx.createBuffer(1, samples.length, TARGET_SR);
  buf.copyToChannel(samples, 0);
  const src = state.ctx.createBufferSource();
  src.buffer = buf;
  src.connect(state.ctx.destination);

  const totalSec = samples.length / TARGET_SR;
  log(`→ ${text}  (${samples.length} samp · ${totalSec.toFixed(1)} s)`, "log-sent");

  state.tx.active = true;
  state.tx.samples = samples;
  state.tx.start = performance.now();
  state.tx.total = samples.length;

  src.onended = () => {
    state.tx.active = false;
    state.tx.samples = null;
  };
  src.start();
  feedTxViz();
}

function feedTxViz() {
  if (!state.tx.active || !state.tx.samples) return;
  // Walk the encoded buffer at wall-clock so the tone bars track speaker output.
  const now = performance.now();
  const elapsed = (now - state.tx.start) / 1000;
  const cursor = Math.min(state.tx.total, Math.floor(elapsed * TARGET_SR));
  const chunkSize = 2400; // 50 ms window
  const start = Math.max(0, cursor - chunkSize);
  if (cursor > start) {
    pushTones(state.tx.samples.subarray(start, cursor));
    scheduleDraw();
  }
  if (state.tx.active) {
    setTimeout(feedTxViz, 50);
  }
}

async function startListen() {
  await ensureCtx();
  await state.ctx.resume();
  els.mic.textContent = "requesting mic…";
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        echoCancellation: false,
        autoGainControl: false,
        noiseSuppression: false,
        channelCount: 1,
      },
    });
  } catch (e) {
    els.mic.textContent = "mic denied";
    log("✗ mic permission denied: " + e.message, "log-dropped");
    return;
  }
  state.rx = new WasmReceiver(getProfile());
  state.micNode = state.ctx.createMediaStreamSource(stream);
  state.workletNode = new AudioWorkletNode(state.ctx, "capture");
  state.workletNode.port.onmessage = (e) => {
    const chunk = e.data;
    // Update tone bars for the live mic (only when not transmitting; otherwise
    // the speaker bleed dominates).
    if (!state.tx.active) {
      pushTones(chunk);
      scheduleDraw();
    }
    const events = state.rx.pushSamples(chunk);
    for (const ev of events) handleEvent(ev);
  };
  state.micNode.connect(state.workletNode);
  // Note: do NOT connect workletNode to ctx.destination — it would loop the mic to the speakers.
  state.listening = true;
  els.listen.textContent = "Stop listening";
  els.listen.classList.add("listening");
  els.mic.textContent = "mic live";
  log(`◉ listening on ${getProfile()}`, "log-info");
}

function stopListen() {
  if (state.micNode) {
    try { state.micNode.disconnect(); } catch {}
    // Stop the underlying mic tracks so the browser indicator goes away.
    const stream = state.micNode.mediaStream;
    if (stream) for (const t of stream.getTracks()) t.stop();
  }
  if (state.workletNode) {
    try { state.workletNode.disconnect(); } catch {}
  }
  state.micNode = null;
  state.workletNode = null;
  state.rx = null;
  state.listening = false;
  els.listen.textContent = "Start listening";
  els.listen.classList.remove("listening");
  els.mic.textContent = "mic idle";
  log("◌ stopped listening", "log-info");
}

function handleEvent(ev) {
  switch (ev.kind) {
    case "frame_ok":
      log(`  frame ${ev.seq} ok (${ev.payload_bytes} B)`, "log-info");
      break;
    case "frame_dropped":
      log(`  frame ${ev.seq} dropped: ${ev.reason}`, "log-dropped");
      break;
    case "stream_complete": {
      const bytes = ev.bytes;
      // serde-wasm-bindgen serializes Vec<u8> as an array of numbers; convert.
      const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
      let text;
      try { text = new TextDecoder("utf-8", { fatal: false }).decode(u8); }
      catch { text = `<${u8.length} bytes>`; }
      const cls = ev.sha256_ok ? "log-recv" : "log-recv bad";
      const mark = ev.sha256_ok ? "←" : "←!";
      log(`${mark} ${text}`, cls);
      break;
    }
  }
}

async function main() {
  try {
    await init();
  } catch (e) {
    els.status.textContent = "wasm load failed";
    log("✗ wasm load failed: " + e.message, "log-dropped");
    return;
  }
  els.status.textContent = `wasm ready · sampleRate=${wasmSampleRate()} Hz`;
  refreshFreqLabels();
  drawTones();

  els.send.addEventListener("click", send);
  els.msg.addEventListener("keydown", (e) => {
    if (e.key === "Enter") send();
  });
  els.listen.addEventListener("click", () => {
    if (state.listening) stopListen(); else startListen();
  });
  document.querySelectorAll('input[name="profile"]').forEach((r) => {
    r.addEventListener("change", () => {
      refreshFreqLabels();
      if (state.listening) {
        // Restart receiver with the new profile.
        const wasListening = state.listening;
        stopListen();
        if (wasListening) startListen();
      }
    });
  });
}

main();
