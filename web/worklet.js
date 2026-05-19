// AudioWorklet capture processor: accumulates input into ~50 ms (2400 sample)
// chunks and posts them to the main thread for the wasm receiver. Runs on
// the realtime audio thread so we don't drop samples under main-thread load.

const CHUNK = 2400; // 50 ms @ 48 kHz

class CaptureProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.buf = new Float32Array(CHUNK);
    this.pos = 0;
  }

  process(inputs) {
    const input = inputs[0];
    if (!input || input.length === 0) return true;
    const ch0 = input[0];
    if (!ch0) return true;
    for (let i = 0; i < ch0.length; i++) {
      this.buf[this.pos++] = ch0[i];
      if (this.pos === CHUNK) {
        // Slice (copy) before posting — the underlying buffer is reused.
        this.port.postMessage(this.buf.slice(0));
        this.pos = 0;
      }
    }
    return true;
  }
}

registerProcessor("capture", CaptureProcessor);
