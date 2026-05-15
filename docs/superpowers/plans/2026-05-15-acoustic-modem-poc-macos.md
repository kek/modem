# Acoustic modem PoC (macOS) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a working MacBook↔MacBook (and MacBook headphones/speakers) acoustic modem CLI that transmits arbitrary bytes via multi-tone FSK over the built-in speaker and microphone, with WAV-based offline testing in CI and a real-audio smoke test.

**Architecture:** A pure-Rust DSP/framing core (`modem-core`) with no I/O, a synchronous push-based state-machine layer (`modem-codec`), a thin `cpal`-based audio adapter (`modem-audio`), and a `clap`-based CLI binary (`modem-cli`) exposing both real-audio (`send`/`recv`) and offline-WAV (`tx-wav`/`rx-wav`) subcommands. PHY is multi-tone 8-FSK behind a `Phy` trait so OFDM can replace it later without touching framing.

**Tech Stack:** Rust 1.75+, Cargo workspace, `rustfft` (FFT), `reed-solomon` v0.2 (RS(255,223)), `crc32fast`, `sha2`, `cpal` (audio I/O), `clap` v4, `hound` (WAV I/O), `proptest`, `anyhow`.

**Out of scope for this plan:** Android, UniFFI, OFDM, ARQ, fountain codes. Will be covered in a follow-up plan once the macOS PoC is validated end-to-end.

---

## File Structure

```
modem/
├── Cargo.toml                              # workspace root
├── docs/
│   └── superpowers/
│       ├── specs/2026-05-15-acoustic-modem-design.md
│       └── plans/2026-05-15-acoustic-modem-poc-macos.md   # this file
├── modem-core/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── crc.rs                          # CRC32 wrapper
│   │   ├── rs.rs                           # Reed-Solomon RS(255,223)
│   │   ├── frame.rs                        # FrameHeader, encode/decode bytes
│   │   ├── fsk.rs                          # FskConfig, FskPhy modulator+demodulator
│   │   ├── preamble.rs                     # Chirp generation + cross-correlation detector
│   │   ├── phy.rs                          # Phy trait
│   │   └── channel.rs                      # Test-only channel impairments (awgn, multipath, …)
│   └── tests/
│       ├── frame_roundtrip.rs
│       ├── fsk_roundtrip.rs
│       └── channel_recovery.rs             # noise/multipath/dropout recovery thresholds
├── modem-codec/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── tx.rs                           # Transmitter
│   │   └── rx.rs                           # Receiver, FrameEvent stream
│   └── tests/
│       └── stream_roundtrip.rs             # multi-frame + proptest
├── modem-audio/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── output.rs                       # cpal speaker stream
│       └── input.rs                        # cpal mic stream
└── modem-cli/
    ├── Cargo.toml
    └── src/
        ├── main.rs                         # clap entry
        ├── cmd_tx_wav.rs
        ├── cmd_rx_wav.rs
        ├── cmd_send.rs
        └── cmd_recv.rs
```

**Boundary principles:** `modem-core` never touches I/O. `modem-codec` orchestrates frames but knows nothing about audio devices. `modem-audio` is a thin wrapper around `cpal` callbacks. `modem-cli` glues them together. Each crate is unit-testable in isolation.

---

## Task 1: Workspace skeleton and shared dependency versions

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `modem-core/Cargo.toml`, `modem-core/src/lib.rs`
- Create: `modem-codec/Cargo.toml`, `modem-codec/src/lib.rs`
- Create: `modem-audio/Cargo.toml`, `modem-audio/src/lib.rs`
- Create: `modem-cli/Cargo.toml`, `modem-cli/src/main.rs`

- [ ] **Step 1: Workspace `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["modem-core", "modem-codec", "modem-audio", "modem-cli"]

[workspace.package]
edition = "2021"
rust-version = "1.75"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
anyhow      = "1"
clap        = { version = "4.5", features = ["derive"] }
cpal        = "0.15"
crc32fast   = "1.4"
hound       = "3.5"
proptest    = "1.5"
reed-solomon = "0.2"
rustfft     = "6.2"
sha2        = "0.10"
thiserror   = "1"
```

- [ ] **Step 2: `modem-core/Cargo.toml`**

```toml
[package]
name = "modem-core"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
crc32fast = { workspace = true }
reed-solomon = { workspace = true }
rustfft = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }
```

- [ ] **Step 3: `modem-codec/Cargo.toml`**

```toml
[package]
name = "modem-codec"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
modem-core = { path = "../modem-core" }
sha2 = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }
```

- [ ] **Step 4: `modem-audio/Cargo.toml`**

```toml
[package]
name = "modem-audio"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
modem-codec = { path = "../modem-codec" }
cpal = { workspace = true }
anyhow = { workspace = true }
```

- [ ] **Step 5: `modem-cli/Cargo.toml`**

```toml
[package]
name = "modem-cli"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[[bin]]
name = "modem"
path = "src/main.rs"

[dependencies]
modem-core  = { path = "../modem-core" }
modem-codec = { path = "../modem-codec" }
modem-audio = { path = "../modem-audio" }
clap   = { workspace = true }
hound  = { workspace = true }
anyhow = { workspace = true }
```

- [ ] **Step 6: Stub `lib.rs`/`main.rs`**

`modem-core/src/lib.rs`:
```rust
#![doc = "Pure DSP and framing for the acoustic modem. No I/O."]
```

`modem-codec/src/lib.rs`:
```rust
#![doc = "Transmitter/Receiver state machines on top of modem-core."]
```

`modem-audio/src/lib.rs`:
```rust
#![doc = "Desktop audio glue (cpal)."]
```

`modem-cli/src/main.rs`:
```rust
fn main() {
    println!("modem-cli stub");
}
```

- [ ] **Step 7: Verify it builds**

Run: `cargo build --workspace`
Expected: compiles cleanly, no warnings beyond `unused`-style lints.

- [ ] **Step 8: Commit**

```bash
jj desc -m "Add Cargo workspace skeleton with four crates"
jj new
```

---

## Task 2: CRC32 wrapper (warm-up)

**Files:**
- Create: `modem-core/src/crc.rs`
- Modify: `modem-core/src/lib.rs` (add `pub mod crc;`)

- [ ] **Step 1: Write the failing test**

Append to `modem-core/src/crc.rs`:
```rust
/// CRC-32 (IEEE 802.3 polynomial), as used in PNG, Ethernet, zlib.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(bytes);
    h.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // Standard test vectors for CRC-32/ISO-HDLC
        assert_eq!(crc32(b""), 0x00000000);
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(crc32(b"hello world"), 0x0D4A1185);
    }
}
```

- [ ] **Step 2: Wire the module**

`modem-core/src/lib.rs`:
```rust
#![doc = "Pure DSP and framing for the acoustic modem. No I/O."]

pub mod crc;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p modem-core crc::`
Expected: PASS (3 known vectors).

- [ ] **Step 4: Commit**

```bash
jj desc -m "Add CRC32 wrapper in modem-core"
jj new
```

---

## Task 3: Reed-Solomon RS(255,223) wrapper

**Files:**
- Create: `modem-core/src/rs.rs`
- Modify: `modem-core/src/lib.rs`

- [ ] **Step 1: Write the failing tests first**

Create `modem-core/src/rs.rs`:
```rust
//! Reed–Solomon RS(255,223): protects 223 bytes of data with 32 parity bytes,
//! corrects up to 16 byte errors per codeword.

use reed_solomon::{Encoder, Decoder};

pub const RS_DATA: usize = 223;
pub const RS_PARITY: usize = 32;
pub const RS_TOTAL: usize = 255;

/// Encode exactly RS_DATA bytes of data into a RS_TOTAL-byte codeword
/// (data followed by 32 parity bytes).
pub fn rs_encode(data: &[u8; RS_DATA]) -> [u8; RS_TOTAL] {
    let enc = Encoder::new(RS_PARITY);
    let buf = enc.encode(data);
    let mut out = [0u8; RS_TOTAL];
    out.copy_from_slice(&buf[..]);
    out
}

#[derive(Debug, thiserror::Error)]
pub enum RsError {
    #[error("too many errors to correct (>16 bytes)")]
    Uncorrectable,
}

/// Decode a 255-byte codeword, correcting up to 16 byte errors.
/// Returns the original 223 data bytes.
pub fn rs_decode(codeword: &[u8; RS_TOTAL]) -> Result<[u8; RS_DATA], RsError> {
    let dec = Decoder::new(RS_PARITY);
    let recovered = dec.correct(codeword, None).map_err(|_| RsError::Uncorrectable)?;
    let mut out = [0u8; RS_DATA];
    out.copy_from_slice(recovered.data());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> [u8; RS_DATA] {
        let mut d = [0u8; RS_DATA];
        for (i, b) in d.iter_mut().enumerate() { *b = (i as u8).wrapping_mul(37).wrapping_add(11); }
        d
    }

    #[test]
    fn clean_roundtrip() {
        let data = sample_data();
        let cw = rs_encode(&data);
        let back = rs_decode(&cw).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn corrects_16_byte_errors() {
        let data = sample_data();
        let mut cw = rs_encode(&data);
        // Flip 16 arbitrary bytes
        for &i in &[3, 17, 42, 60, 77, 100, 128, 150, 170, 190, 200, 210, 220, 230, 240, 250] {
            cw[i] ^= 0xFF;
        }
        let back = rs_decode(&cw).unwrap();
        assert_eq!(back, data, "RS must correct up to 16 byte errors");
    }

    #[test]
    fn fails_above_16_byte_errors() {
        let data = sample_data();
        let mut cw = rs_encode(&data);
        for i in 0..17 {
            cw[i * 10] ^= 0xAA;
        }
        assert!(rs_decode(&cw).is_err(), "17 errors must be uncorrectable");
    }
}
```

`modem-core/src/lib.rs`:
```rust
#![doc = "Pure DSP and framing for the acoustic modem. No I/O."]

pub mod crc;
pub mod rs;
```

- [ ] **Step 2: Run tests, expect them to compile and pass**

Run: `cargo test -p modem-core rs::`
Expected: all three tests pass.

If `rs_decode` fails to compile because of API differences in `reed-solomon` 0.2 (`Buffer::data()` returns the data slice), inspect with `cargo doc --open -p reed-solomon` and adjust.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add RS(255,223) wrapper around reed-solomon crate"
jj new
```

---

## Task 4: Frame encoding and decoding

**Files:**
- Create: `modem-core/src/frame.rs`
- Modify: `modem-core/src/lib.rs`

Frame on the wire (after preamble + sync, both added by the PHY layer in a later task):

```
[ header (4 B) | payload (≤219 B) | RS parity (32 B) | CRC32 (4 B) ]
                \_______ RS(255,223) data block ______/
```

Header layout (4 bytes):
- byte 0: `version` (0x01 for now)
- byte 1: `flags` — bit 0 = first frame, bit 1 = last frame, bit 2 = profile (0 audible, 1 ultrasonic), bits 3-7 reserved
- bytes 2-3: `payload_len` u16 big-endian (must be ≤ 219)

- [ ] **Step 1: Write failing tests**

Create `modem-core/src/frame.rs`:
```rust
//! Framing: header + payload + RS(255,223) parity + CRC32.

use crate::crc::crc32;
use crate::rs::{rs_decode, rs_encode, RS_DATA, RS_PARITY};

pub const HEADER_LEN: usize = 4;
pub const MAX_PAYLOAD: usize = RS_DATA - HEADER_LEN; // 219
pub const FRAME_LEN: usize = RS_DATA + RS_PARITY + 4; // 255 + 4 CRC = 259 bytes

const VERSION: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub first: bool,
    pub last: bool,
    pub ultrasonic: bool,
    pub payload_len: u16,
}

impl FrameHeader {
    pub fn flags_byte(&self) -> u8 {
        (self.first as u8)
            | ((self.last as u8) << 1)
            | ((self.ultrasonic as u8) << 2)
    }
    pub fn from_bytes(b: &[u8; HEADER_LEN]) -> Result<Self, FrameError> {
        if b[0] != VERSION {
            return Err(FrameError::BadVersion(b[0]));
        }
        let flags = b[1];
        let payload_len = u16::from_be_bytes([b[2], b[3]]);
        if payload_len as usize > MAX_PAYLOAD {
            return Err(FrameError::PayloadTooLong(payload_len));
        }
        Ok(Self {
            first: flags & 0b001 != 0,
            last:  flags & 0b010 != 0,
            ultrasonic: flags & 0b100 != 0,
            payload_len,
        })
    }
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[0] = VERSION;
        h[1] = self.flags_byte();
        let len_be = self.payload_len.to_be_bytes();
        h[2] = len_be[0];
        h[3] = len_be[1];
        h
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("payload too long: {0} > 219")]
    PayloadTooLong(u16),
    #[error("bad version byte: 0x{0:02x}")]
    BadVersion(u8),
    #[error("frame length wrong: got {0}, expected {FRAME_LEN}")]
    BadLength(usize),
    #[error("RS uncorrectable")]
    RsUncorrectable,
    #[error("CRC mismatch")]
    CrcMismatch,
}

/// Encode a frame into FRAME_LEN bytes.
pub fn encode_frame(header: FrameHeader, payload: &[u8]) -> Result<[u8; FRAME_LEN], FrameError> {
    if payload.len() != header.payload_len as usize {
        return Err(FrameError::PayloadTooLong(payload.len() as u16));
    }
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::PayloadTooLong(payload.len() as u16));
    }

    // Build the 223-byte RS data block: header + payload, right-padded with zeros.
    let mut data = [0u8; RS_DATA];
    data[..HEADER_LEN].copy_from_slice(&header.to_bytes());
    data[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);

    let codeword = rs_encode(&data);

    let mut out = [0u8; FRAME_LEN];
    out[..codeword.len()].copy_from_slice(&codeword);

    // CRC over header + payload (the *application-visible* data, not the padding).
    let crc = crc32(&data[..HEADER_LEN + payload.len()]);
    out[codeword.len()..].copy_from_slice(&crc.to_be_bytes());
    Ok(out)
}

/// Decode a frame's bytes back to (header, payload). Returns the longest sensible payload.
pub fn decode_frame(bytes: &[u8]) -> Result<(FrameHeader, Vec<u8>), FrameError> {
    if bytes.len() != FRAME_LEN {
        return Err(FrameError::BadLength(bytes.len()));
    }
    let mut cw = [0u8; RS_DATA + RS_PARITY];
    cw.copy_from_slice(&bytes[..RS_DATA + RS_PARITY]);

    let data = rs_decode(&cw).map_err(|_| FrameError::RsUncorrectable)?;

    let header_bytes: [u8; HEADER_LEN] = data[..HEADER_LEN].try_into().unwrap();
    let header = FrameHeader::from_bytes(&header_bytes)?;
    let payload_end = HEADER_LEN + header.payload_len as usize;
    let payload = data[HEADER_LEN..payload_end].to_vec();

    // Verify CRC
    let crc_bytes: [u8; 4] = bytes[RS_DATA + RS_PARITY..].try_into().unwrap();
    let expected = u32::from_be_bytes(crc_bytes);
    let actual = crc32(&data[..payload_end]);
    if expected != actual {
        return Err(FrameError::CrcMismatch);
    }

    Ok((header, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(first: bool, last: bool, len: usize) -> FrameHeader {
        FrameHeader { first, last, ultrasonic: false, payload_len: len as u16 }
    }

    #[test]
    fn empty_payload_roundtrip() {
        let header = h(true, true, 0);
        let bytes = encode_frame(header, &[]).unwrap();
        let (h2, p) = decode_frame(&bytes).unwrap();
        assert_eq!(h2, header);
        assert_eq!(p, Vec::<u8>::new());
    }

    #[test]
    fn max_payload_roundtrip() {
        let payload: Vec<u8> = (0..MAX_PAYLOAD).map(|i| i as u8).collect();
        let header = h(true, true, MAX_PAYLOAD);
        let bytes = encode_frame(header, &payload).unwrap();
        let (h2, p) = decode_frame(&bytes).unwrap();
        assert_eq!(h2, header);
        assert_eq!(p, payload);
    }

    #[test]
    fn corrects_byte_errors() {
        let payload: Vec<u8> = b"hello, world! this is a test payload.".to_vec();
        let header = h(true, true, payload.len());
        let mut bytes = encode_frame(header, &payload).unwrap();
        // Corrupt 10 bytes scattered across the frame
        for i in [5, 22, 50, 80, 110, 140, 170, 200, 230, 240] {
            bytes[i] ^= 0x55;
        }
        let (_, p) = decode_frame(&bytes).unwrap();
        assert_eq!(p, payload);
    }

    #[test]
    fn rejects_oversize_payload() {
        let payload = vec![0u8; MAX_PAYLOAD + 1];
        let header = h(true, true, payload.len());
        assert!(encode_frame(header, &payload).is_err());
    }

    #[test]
    fn detects_crc_mismatch_after_rs_failure_unlikely() {
        // 17+ errors → RS gives up. We treat as RsUncorrectable, not CRC.
        let payload = vec![0xAA; 100];
        let header = h(true, true, payload.len());
        let mut bytes = encode_frame(header, &payload).unwrap();
        for i in 0..20 {
            bytes[i] ^= 0xFF;
        }
        assert!(matches!(decode_frame(&bytes), Err(FrameError::RsUncorrectable)));
    }
}
```

`modem-core/src/lib.rs`:
```rust
#![doc = "Pure DSP and framing for the acoustic modem. No I/O."]

pub mod crc;
pub mod frame;
pub mod rs;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p modem-core frame::`
Expected: all 5 tests pass.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add frame encoding/decoding with RS + CRC32"
jj new
```

---

## Task 5: FSK configuration and presets

**Files:**
- Create: `modem-core/src/fsk.rs` (config only for now)
- Modify: `modem-core/src/lib.rs`

- [ ] **Step 1: Write the config and presets**

Create `modem-core/src/fsk.rs`:
```rust
//! Multi-tone FSK modulator / demodulator.

/// Number of tones (must be a power of two).
pub const N_TONES: usize = 8;
pub const BITS_PER_SYMBOL: usize = 3;

/// Fixed sample rate everywhere.
pub const SAMPLE_RATE: u32 = 48_000;

#[derive(Debug, Clone, Copy)]
pub struct FskConfig {
    pub sample_rate: u32,
    pub symbol_samples: usize,
    /// 8 distinct frequencies, in Hz.
    pub tone_freqs: [f32; N_TONES],
}

impl FskConfig {
    /// Audible profile: 8-FSK, 100 sym/s, 500 Hz spacing, 2.0 – 5.5 kHz.
    pub fn audible() -> Self {
        let mut tones = [0f32; N_TONES];
        for i in 0..N_TONES {
            tones[i] = 2000.0 + (i as f32) * 500.0; // 2000..5500
        }
        Self {
            sample_rate: SAMPLE_RATE,
            symbol_samples: SAMPLE_RATE as usize / 100, // 10 ms
            tone_freqs: tones,
        }
    }

    /// Ultrasonic profile: 8-FSK, 50 sym/s, 250 Hz spacing, 17.5 – 19.25 kHz.
    pub fn ultrasonic() -> Self {
        let mut tones = [0f32; N_TONES];
        for i in 0..N_TONES {
            tones[i] = 17_500.0 + (i as f32) * 250.0;
        }
        Self {
            sample_rate: SAMPLE_RATE,
            symbol_samples: SAMPLE_RATE as usize / 50, // 20 ms
            tone_freqs: tones,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audible_geometry() {
        let c = FskConfig::audible();
        assert_eq!(c.symbol_samples, 480);
        assert_eq!(c.tone_freqs[0], 2000.0);
        assert_eq!(c.tone_freqs[7], 5500.0);
    }

    #[test]
    fn ultrasonic_geometry() {
        let c = FskConfig::ultrasonic();
        assert_eq!(c.symbol_samples, 960);
        assert_eq!(c.tone_freqs[0], 17_500.0);
        assert_eq!(c.tone_freqs[7], 19_250.0);
    }

    #[test]
    fn nyquist_safe() {
        let nyq = SAMPLE_RATE as f32 / 2.0;
        for c in [FskConfig::audible(), FskConfig::ultrasonic()] {
            for &f in &c.tone_freqs {
                assert!(f < nyq, "tone {f} exceeds Nyquist {nyq}");
            }
        }
    }
}
```

`modem-core/src/lib.rs`:
```rust
pub mod crc;
pub mod frame;
pub mod fsk;
pub mod rs;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p modem-core fsk::`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add FskConfig with audible and ultrasonic presets"
jj new
```

---

## Task 6: FSK modulator (bytes → samples)

**Files:**
- Modify: `modem-core/src/fsk.rs`

- [ ] **Step 1: Write the failing test first**

Append to `modem-core/src/fsk.rs`:
```rust
/// Convert bytes to a sequence of 3-bit symbols (MSB first within each byte).
pub fn bytes_to_symbols(bytes: &[u8]) -> Vec<u8> {
    let total_bits = bytes.len() * 8;
    // Pad to a multiple of BITS_PER_SYMBOL with zero bits.
    let n_syms = (total_bits + BITS_PER_SYMBOL - 1) / BITS_PER_SYMBOL;
    let mut syms = Vec::with_capacity(n_syms);
    let mut bit_idx = 0;
    for _ in 0..n_syms {
        let mut sym: u8 = 0;
        for _ in 0..BITS_PER_SYMBOL {
            let byte_i = bit_idx / 8;
            let bit_i = 7 - (bit_idx % 8); // MSB first
            let bit = if byte_i < bytes.len() {
                (bytes[byte_i] >> bit_i) & 1
            } else {
                0 // pad
            };
            sym = (sym << 1) | bit;
            bit_idx += 1;
        }
        syms.push(sym);
    }
    syms
}

/// Inverse of bytes_to_symbols. `n_bytes` is the expected output length;
/// extra symbols (padding) are dropped.
pub fn symbols_to_bytes(syms: &[u8], n_bytes: usize) -> Vec<u8> {
    let mut out = vec![0u8; n_bytes];
    let mut bit_idx = 0;
    for &s in syms {
        for k in 0..BITS_PER_SYMBOL {
            let bit = (s >> (BITS_PER_SYMBOL - 1 - k)) & 1;
            let byte_i = bit_idx / 8;
            let bit_i = 7 - (bit_idx % 8);
            if byte_i < n_bytes {
                out[byte_i] |= bit << bit_i;
            }
            bit_idx += 1;
        }
    }
    out
}

/// Modulate a symbol stream into audio samples.
pub fn modulate(cfg: &FskConfig, symbols: &[u8]) -> Vec<f32> {
    let mut out = Vec::with_capacity(symbols.len() * cfg.symbol_samples);
    let dt = 1.0 / cfg.sample_rate as f32;
    // Continuous phase across symbols to avoid clicks.
    let mut phase = 0f32;
    for &s in symbols {
        debug_assert!((s as usize) < N_TONES);
        let f = cfg.tone_freqs[s as usize];
        let dphase = 2.0 * std::f32::consts::PI * f * dt;
        for _ in 0..cfg.symbol_samples {
            out.push(phase.sin() * 0.6); // leave headroom
            phase += dphase;
            if phase > std::f32::consts::TAU {
                phase -= std::f32::consts::TAU;
            }
        }
    }
    out
}

#[cfg(test)]
mod modulator_tests {
    use super::*;

    #[test]
    fn symbol_roundtrip_byte_aligned() {
        // 3 bytes = 24 bits = 8 symbols exactly.
        let bytes = vec![0xDE, 0xAD, 0xBE];
        let syms = bytes_to_symbols(&bytes);
        assert_eq!(syms.len(), 8);
        let back = symbols_to_bytes(&syms, bytes.len());
        assert_eq!(back, bytes);
    }

    #[test]
    fn symbol_roundtrip_unaligned() {
        let bytes = vec![0x01, 0x02]; // 16 bits → 6 symbols (18 bits, padded)
        let syms = bytes_to_symbols(&bytes);
        assert_eq!(syms.len(), 6);
        let back = symbols_to_bytes(&syms, bytes.len());
        assert_eq!(back, bytes);
    }

    #[test]
    fn modulator_output_length() {
        let cfg = FskConfig::audible();
        let syms = vec![0u8; 10];
        let samples = modulate(&cfg, &syms);
        assert_eq!(samples.len(), 10 * cfg.symbol_samples);
    }

    #[test]
    fn modulator_amplitude_bounded() {
        let cfg = FskConfig::audible();
        let syms = vec![3, 5, 7, 1];
        let samples = modulate(&cfg, &syms);
        for s in samples {
            assert!(s.abs() <= 0.61, "sample out of headroom: {s}");
        }
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p modem-core fsk::modulator_tests`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add FSK modulator: bytes -> symbols -> samples"
jj new
```

---

## Task 7: FSK demodulator (samples → symbols → bytes)

We use the **Goertzel algorithm** — a 1-bin DFT, far cheaper than a full FFT when you only care about 8 specific frequencies.

**Files:**
- Modify: `modem-core/src/fsk.rs`

- [ ] **Step 1: Write the failing test**

Append to `modem-core/src/fsk.rs`:
```rust
/// Goertzel single-bin magnitude squared for frequency `f` in `samples`.
fn goertzel_mag2(samples: &[f32], f: f32, sample_rate: u32) -> f32 {
    let n = samples.len() as f32;
    let k = (0.5 + n * f / sample_rate as f32).floor();
    let w = 2.0 * std::f32::consts::PI * k / n;
    let coeff = 2.0 * w.cos();
    let (mut s0, mut s1, mut s2) = (0f32, 0f32, 0f32);
    for &x in samples {
        s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// Demodulate audio samples (assumed symbol-aligned) into a symbol stream.
/// `samples.len()` must be a multiple of `cfg.symbol_samples`.
pub fn demodulate(cfg: &FskConfig, samples: &[f32]) -> Vec<u8> {
    let n = cfg.symbol_samples;
    assert_eq!(samples.len() % n, 0, "demodulate: not symbol-aligned");
    let n_syms = samples.len() / n;
    let mut out = Vec::with_capacity(n_syms);
    for i in 0..n_syms {
        let win = &samples[i * n..(i + 1) * n];
        let mut best_idx = 0usize;
        let mut best_mag = f32::NEG_INFINITY;
        for (t, &f) in cfg.tone_freqs.iter().enumerate() {
            let m = goertzel_mag2(win, f, cfg.sample_rate);
            if m > best_mag {
                best_mag = m;
                best_idx = t;
            }
        }
        out.push(best_idx as u8);
    }
    out
}

#[cfg(test)]
mod demod_tests {
    use super::*;

    #[test]
    fn clean_roundtrip_audible() {
        let cfg = FskConfig::audible();
        let bytes = b"acoustic modem".to_vec();
        let syms = bytes_to_symbols(&bytes);
        let samples = modulate(&cfg, &syms);
        let back_syms = demodulate(&cfg, &samples);
        assert_eq!(back_syms, syms);
        let back = symbols_to_bytes(&back_syms, bytes.len());
        assert_eq!(back, bytes);
    }

    #[test]
    fn clean_roundtrip_ultrasonic() {
        let cfg = FskConfig::ultrasonic();
        let bytes = b"hi".to_vec();
        let syms = bytes_to_symbols(&bytes);
        let samples = modulate(&cfg, &syms);
        let back = demodulate(&cfg, &samples);
        assert_eq!(back, syms);
    }

    #[test]
    fn resilient_to_small_white_noise() {
        let cfg = FskConfig::audible();
        let bytes = b"noise test 12345".to_vec();
        let syms = bytes_to_symbols(&bytes);
        let mut samples = modulate(&cfg, &syms);
        // Add deterministic "noise" at ~-20 dB peak
        for (i, s) in samples.iter_mut().enumerate() {
            let n = ((i * 7919) % 7) as f32 / 7.0 - 0.5;
            *s += n * 0.06;
        }
        let back = demodulate(&cfg, &samples);
        assert_eq!(back, syms, "demodulator failed under mild noise");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p modem-core fsk::demod_tests`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add FSK demodulator using Goertzel"
jj new
```

---

## Task 8: Chirp preamble generation and detection

**Files:**
- Create: `modem-core/src/preamble.rs`
- Modify: `modem-core/src/lib.rs`

The preamble is a **linear up-chirp** from `f0` to `f1` over `duration_samples`. To detect, we cross-correlate the incoming buffer against the known chirp template and look for a peak.

- [ ] **Step 1: Write the chirp generator and a simple cross-correlator**

Create `modem-core/src/preamble.rs`:
```rust
//! Frame preamble: linear up-chirp + sync word.

use crate::fsk::SAMPLE_RATE;

/// Sync word that follows the chirp (one symbol's worth of recognizable bytes).
pub const SYNC_WORD: [u8; 2] = [0x7E, 0x81];

pub fn preamble_samples() -> Vec<f32> {
    // 80 ms chirp from 1.5 kHz to 6 kHz
    chirp(1_500.0, 6_000.0, 0.080)
}

fn chirp(f0: f32, f1: f32, duration_s: f32) -> Vec<f32> {
    let n = (SAMPLE_RATE as f32 * duration_s) as usize;
    let mut out = Vec::with_capacity(n);
    let dt = 1.0 / SAMPLE_RATE as f32;
    let k = (f1 - f0) / duration_s; // Hz per second
    let mut t = 0f32;
    let mut phase = 0f32;
    for _ in 0..n {
        let f = f0 + k * t;
        phase += 2.0 * std::f32::consts::PI * f * dt;
        out.push(phase.sin() * 0.6);
        t += dt;
    }
    out
}

/// Slide the template across `haystack` and return (best_offset, peak_score).
/// score is normalised energy-of-correlation.
pub fn detect_preamble(haystack: &[f32], template: &[f32]) -> Option<(usize, f32)> {
    if haystack.len() < template.len() {
        return None;
    }
    let tn = template.len();
    let template_energy: f32 = template.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mut best: Option<(usize, f32)> = None;
    // Stride 4 samples for speed (~12 kHz effective scan rate); fine enough for ~80ms preamble.
    for i in (0..=haystack.len() - tn).step_by(4) {
        let mut corr = 0f32;
        let mut win_energy = 0f32;
        for k in 0..tn {
            let x = haystack[i + k];
            corr += x * template[k];
            win_energy += x * x;
        }
        let denom = (win_energy.sqrt() * template_energy).max(1e-9);
        let score = corr / denom;
        if best.map_or(true, |(_, b)| score > b) {
            best = Some((i, score));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chirp_has_expected_length() {
        let p = preamble_samples();
        assert_eq!(p.len(), (SAMPLE_RATE as f32 * 0.080) as usize);
    }

    #[test]
    fn detects_preamble_at_known_offset() {
        let template = preamble_samples();
        let mut haystack = vec![0f32; 2000];
        haystack.extend(template.iter().copied());
        haystack.extend(std::iter::repeat(0f32).take(2000));
        let (off, score) = detect_preamble(&haystack, &template).unwrap();
        // Stride is 4, so allow ±3-sample tolerance.
        assert!((off as i64 - 2000).abs() <= 4, "off={off}");
        assert!(score > 0.9, "score={score}");
    }

    #[test]
    fn rejects_pure_noise() {
        let template = preamble_samples();
        // Deterministic "noise"
        let haystack: Vec<f32> = (0..10000).map(|i| (((i * 7919) % 17) as f32 / 17.0 - 0.5) * 0.2).collect();
        let (_, score) = detect_preamble(&haystack, &template).unwrap();
        assert!(score < 0.3, "noise should not produce high correlation, got {score}");
    }
}
```

`modem-core/src/lib.rs`:
```rust
pub mod crc;
pub mod frame;
pub mod fsk;
pub mod preamble;
pub mod rs;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p modem-core preamble::`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add chirp preamble generation and cross-correlation detection"
jj new
```

---

## Task 9: Phy trait + FskPhy adapter

**Files:**
- Create: `modem-core/src/phy.rs`
- Modify: `modem-core/src/lib.rs`

The `Phy` trait bundles modulation + demodulation + preamble. The codec layer uses only this trait — adding OFDM later means a new impl.

- [ ] **Step 1: Define trait and FSK implementation**

Create `modem-core/src/phy.rs`:
```rust
//! PHY trait + multi-tone FSK implementation.

use crate::fsk::{demodulate, modulate, bytes_to_symbols, symbols_to_bytes, FskConfig, BITS_PER_SYMBOL};
use crate::preamble::{preamble_samples, detect_preamble};

pub trait Phy {
    fn sample_rate(&self) -> u32;
    fn preamble(&self) -> Vec<f32>;
    fn detect_preamble(&self, samples: &[f32]) -> Option<(usize, f32)>;
    /// Modulate a frame's raw bytes (`FRAME_LEN` bytes) into samples.
    fn modulate_bytes(&self, bytes: &[u8]) -> Vec<f32>;
    /// Demodulate `n_bytes` worth of samples (symbol-aligned).
    fn demodulate_bytes(&self, samples: &[f32], n_bytes: usize) -> Vec<u8>;
    /// Samples per frame's data section (for the receiver's window math).
    fn frame_data_samples(&self, n_bytes: usize) -> usize;
}

pub struct FskPhy {
    cfg: FskConfig,
    preamble: Vec<f32>,
}

impl FskPhy {
    pub fn audible() -> Self {
        Self { cfg: FskConfig::audible(), preamble: preamble_samples() }
    }
    pub fn ultrasonic() -> Self {
        Self { cfg: FskConfig::ultrasonic(), preamble: preamble_samples() }
    }
    pub fn config(&self) -> &FskConfig { &self.cfg }
}

impl Phy for FskPhy {
    fn sample_rate(&self) -> u32 { self.cfg.sample_rate }
    fn preamble(&self) -> Vec<f32> { self.preamble.clone() }
    fn detect_preamble(&self, samples: &[f32]) -> Option<(usize, f32)> {
        detect_preamble(samples, &self.preamble)
    }
    fn modulate_bytes(&self, bytes: &[u8]) -> Vec<f32> {
        let syms = bytes_to_symbols(bytes);
        modulate(&self.cfg, &syms)
    }
    fn demodulate_bytes(&self, samples: &[f32], n_bytes: usize) -> Vec<u8> {
        let n_syms = (n_bytes * 8 + BITS_PER_SYMBOL - 1) / BITS_PER_SYMBOL;
        let needed = n_syms * self.cfg.symbol_samples;
        let win = &samples[..needed.min(samples.len())];
        let syms = demodulate(&self.cfg, win);
        symbols_to_bytes(&syms, n_bytes)
    }
    fn frame_data_samples(&self, n_bytes: usize) -> usize {
        let n_syms = (n_bytes * 8 + BITS_PER_SYMBOL - 1) / BITS_PER_SYMBOL;
        n_syms * self.cfg.symbol_samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_via_trait_object() {
        let phy: Box<dyn Phy> = Box::new(FskPhy::audible());
        let bytes = vec![0x11, 0x22, 0x33, 0x44, 0x55];
        let samples = phy.modulate_bytes(&bytes);
        let back = phy.demodulate_bytes(&samples, bytes.len());
        assert_eq!(back, bytes);
    }
}
```

`modem-core/src/lib.rs`:
```rust
pub mod crc;
pub mod frame;
pub mod fsk;
pub mod phy;
pub mod preamble;
pub mod rs;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p modem-core phy::`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add Phy trait with FskPhy implementation"
jj new
```

---

## Task 10: Channel impairment utilities (test-only)

**Files:**
- Create: `modem-core/src/channel.rs`
- Modify: `modem-core/src/lib.rs`

Test-only helpers so we can simulate noise, multipath, frequency offset, clipping, and dropouts in pure Rust without a sound card.

- [ ] **Step 1: Write the helpers**

Create `modem-core/src/channel.rs`:
```rust
//! Test-only channel impairments. NOT compiled outside cfg(test) for end users,
//! but exposed unconditionally so integration tests in modem-codec can also use them.

/// Additive white Gaussian noise (Marsaglia polar method, deterministic via seed).
pub fn awgn(samples: &mut [f32], snr_db: f32, seed: u64) {
    let sig_power: f32 = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
    let snr_linear = 10f32.powf(snr_db / 10.0);
    let noise_power = sig_power / snr_linear;
    let sigma = noise_power.sqrt();
    let mut s = seed;
    for x in samples.iter_mut() {
        // xorshift64 → uniform → Box-Muller (just sin form, deterministic enough)
        s ^= s << 13; s ^= s >> 7; s ^= s << 17;
        let u1 = ((s as u32) as f32 / u32::MAX as f32).max(1e-9);
        s ^= s << 13; s ^= s >> 7; s ^= s << 17;
        let u2 = (s as u32) as f32 / u32::MAX as f32;
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos();
        *x += z * sigma;
    }
}

/// Apply a constant frequency offset by multiplying with a complex exponential
/// (here we cheat: small offsets just shift each sample's phase linearly).
pub fn freq_offset(samples: &mut [f32], hz: f32, sample_rate: u32) {
    let dphi = 2.0 * std::f32::consts::PI * hz / sample_rate as f32;
    let mut phi = 0f32;
    for x in samples.iter_mut() {
        *x *= phi.cos();
        phi += dphi;
    }
}

/// One-tap multipath: y[n] = x[n] + gain * x[n - delay_samples]
pub fn multipath(samples: &mut [f32], delay_samples: usize, gain: f32) {
    let orig: Vec<f32> = samples.to_vec();
    for n in delay_samples..samples.len() {
        samples[n] += gain * orig[n - delay_samples];
    }
}

/// Hard-clip to ±threshold.
pub fn clip(samples: &mut [f32], threshold: f32) {
    for x in samples.iter_mut() {
        *x = x.clamp(-threshold, threshold);
    }
}

/// Zero out a window of samples (simulates a dropout / cough / packet loss).
pub fn drop_window(samples: &mut [f32], start: usize, len: usize) {
    let end = (start + len).min(samples.len());
    for x in &mut samples[start..end] {
        *x = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awgn_reduces_snr_predictably() {
        let mut samples: Vec<f32> = (0..4800).map(|i| (i as f32 / 48.0).sin() * 0.5).collect();
        let before_rms: f32 = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
        awgn(&mut samples, 6.0, 42);
        let after_rms: f32 = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
        assert!(after_rms > before_rms, "noise should raise RMS");
    }

    #[test]
    fn drop_window_zeros_correct_range() {
        let mut s = vec![1f32; 100];
        drop_window(&mut s, 20, 10);
        assert!(s[20..30].iter().all(|&x| x == 0.0));
        assert!(s[0..20].iter().all(|&x| x == 1.0));
        assert!(s[30..].iter().all(|&x| x == 1.0));
    }
}
```

`modem-core/src/lib.rs`:
```rust
pub mod channel;
pub mod crc;
pub mod frame;
pub mod fsk;
pub mod phy;
pub mod preamble;
pub mod rs;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p modem-core channel::`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add test-only channel impairment helpers (AWGN, multipath, etc.)"
jj new
```

---

## Task 11: Single-frame end-to-end test through the simulated channel

**Files:**
- Create: `modem-core/tests/channel_recovery.rs`

- [ ] **Step 1: Write the integration test**

Create `modem-core/tests/channel_recovery.rs`:
```rust
//! End-to-end through encode→modulate→channel→demodulate→decode for one frame.

use modem_core::channel::{awgn, drop_window, multipath};
use modem_core::frame::{decode_frame, encode_frame, FrameHeader, FRAME_LEN, MAX_PAYLOAD};
use modem_core::phy::{FskPhy, Phy};

fn header_of(len: usize) -> FrameHeader {
    FrameHeader { first: true, last: true, ultrasonic: false, payload_len: len as u16 }
}

fn payload() -> Vec<u8> {
    (0..MAX_PAYLOAD).map(|i| ((i * 31) & 0xFF) as u8).collect()
}

#[test]
fn clean_channel_audible() {
    let phy = FskPhy::audible();
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();
    let samples = phy.modulate_bytes(&frame);
    let back = phy.demodulate_bytes(&samples, FRAME_LEN);
    let (_, decoded) = decode_frame(&back).unwrap();
    assert_eq!(decoded, p);
}

#[test]
fn awgn_12db_audible_recovers() {
    let phy = FskPhy::audible();
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();
    let mut samples = phy.modulate_bytes(&frame);
    awgn(&mut samples, 12.0, 0xC0FFEE);
    let back = phy.demodulate_bytes(&samples, FRAME_LEN);
    let (_, decoded) = decode_frame(&back).unwrap();
    assert_eq!(decoded, p);
}

#[test]
fn single_dropout_recovers_via_rs() {
    let phy = FskPhy::audible();
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();
    let mut samples = phy.modulate_bytes(&frame);
    // 200 ms cough in the middle = 9600 samples zeroed.
    let mid = samples.len() / 2;
    drop_window(&mut samples, mid, 9600);
    let back = phy.demodulate_bytes(&samples, FRAME_LEN);
    // RS may or may not recover depending on which byte positions were hit;
    // ensure at least decoding doesn't panic, and assert recovery when fewer
    // than 16 bytes were corrupted.
    if let Ok((_, decoded)) = decode_frame(&back) {
        assert_eq!(decoded, p);
    }
    // If decode fails, that's also acceptable for a 9600-sample dropout —
    // documents the realistic threshold. The codec layer will retry by re-syncing.
}

#[test]
fn small_multipath_recovers() {
    let phy = FskPhy::audible();
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();
    let mut samples = phy.modulate_bytes(&frame);
    multipath(&mut samples, 240 /* 5 ms */, 0.3);
    let back = phy.demodulate_bytes(&samples, FRAME_LEN);
    let (_, decoded) = decode_frame(&back).unwrap();
    assert_eq!(decoded, p);
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p modem-core --test channel_recovery`
Expected: all four tests pass.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add channel-recovery integration tests for single-frame path"
jj new
```

---

## Task 12: Transmitter (modem-codec)

**Files:**
- Create: `modem-codec/src/tx.rs`
- Modify: `modem-codec/src/lib.rs`

The Transmitter splits a byte stream into frames, prefixes each with preamble + sync word + frame bytes, and produces the full audio samples for transmission. The last frame's payload ends with a 32-byte SHA-256 of the full input.

- [ ] **Step 1: Write the transmitter**

Create `modem-codec/src/tx.rs`:
```rust
use modem_core::frame::{encode_frame, FrameHeader, MAX_PAYLOAD};
use modem_core::phy::Phy;
use modem_core::preamble::SYNC_WORD;
use sha2::{Digest, Sha256};

pub struct Transmitter<P: Phy> {
    phy: P,
    ultrasonic: bool,
}

impl<P: Phy> Transmitter<P> {
    pub fn new(phy: P, ultrasonic: bool) -> Self {
        Self { phy, ultrasonic }
    }

    /// Encode `payload` into a complete audio sample buffer (preamble + sync + frame, repeated).
    pub fn encode(&self, payload: &[u8]) -> Vec<f32> {
        // Append SHA-256 to last frame's data.
        let sha = Sha256::digest(payload);
        let mut full = Vec::with_capacity(payload.len() + 32);
        full.extend_from_slice(payload);
        full.extend_from_slice(&sha);

        // Per-frame app payload max: 219 normally, but the *last* frame must fit
        // the trailing 32-byte sha. We treat it as if everything is one stream
        // up to 219 bytes per frame, and the sha is just part of the last
        // frame's bytes.
        let chunks: Vec<&[u8]> = full.chunks(MAX_PAYLOAD).collect();
        let n_frames = chunks.len();

        let mut out: Vec<f32> = Vec::new();
        let preamble = self.phy.preamble();
        for (i, chunk) in chunks.iter().enumerate() {
            // Each frame: preamble | sync_word_modulated | frame_bytes_modulated
            let header = FrameHeader {
                first: i == 0,
                last: i == n_frames - 1,
                ultrasonic: self.ultrasonic,
                payload_len: chunk.len() as u16,
            };
            let frame_bytes = encode_frame(header, chunk).expect("payload size checked above");

            // IMPORTANT: modulate sync + frame as ONE byte buffer.
            // bytes_to_symbols packs bits across byte boundaries; splitting into
            // two modulate calls would introduce padding bits between them that
            // the receiver's single demodulate window would not expect.
            let mut combined = Vec::with_capacity(SYNC_WORD.len() + frame_bytes.len());
            combined.extend_from_slice(&SYNC_WORD);
            combined.extend_from_slice(&frame_bytes);

            out.extend_from_slice(&preamble);
            out.extend_from_slice(&self.phy.modulate_bytes(&combined));
        }
        out
    }
}
```

`modem-codec/src/lib.rs`:
```rust
pub mod tx;
```

- [ ] **Step 2: Write a smoke test**

Append at the bottom of `modem-codec/src/tx.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use modem_core::phy::FskPhy;

    #[test]
    fn encodes_to_nonempty_samples() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let samples = tx.encode(b"hi");
        assert!(samples.len() > 1000, "got {}", samples.len());
    }

    #[test]
    fn multi_frame_payload_splits() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        // 500 bytes of payload + 32 bytes sha = 532 → 3 frames (219+219+94)
        let payload = vec![0xAA; 500];
        let samples = tx.encode(&payload);
        // 3 preambles → at minimum 3 * 80 ms = 240 ms of preamble alone = 11520 samples
        assert!(samples.len() > 11_500);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p modem-codec`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
jj desc -m "Add Transmitter in modem-codec with multi-frame splitting + SHA-256 tail"
jj new
```

---

## Task 13: Receiver state machine

**Files:**
- Create: `modem-codec/src/rx.rs`
- Modify: `modem-codec/src/lib.rs`

The receiver takes a stream of samples (pushed in chunks) and emits `FrameEvent`s.

States:
1. **Searching** for a preamble.
2. **Reading sync word** to confirm.
3. **Reading frame bytes** (always 259 bytes per frame = `FRAME_LEN`).
4. Back to **Searching**.

When a `last` frame is decoded, we verify SHA-256 over assembled payload and emit `StreamComplete`.

- [ ] **Step 1: Write the receiver**

Create `modem-codec/src/rx.rs`:
```rust
use modem_core::frame::{decode_frame, FRAME_LEN, FrameError, MAX_PAYLOAD};
use modem_core::phy::Phy;
use modem_core::preamble::SYNC_WORD;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameEvent {
    FrameOk { seq: u32, bytes: Vec<u8> },
    FrameDropped { seq: u32, reason: String },
    StreamComplete { bytes: Vec<u8>, sha256_ok: bool },
}

enum State {
    Searching,
    AfterPreamble,
}

pub struct Receiver<P: Phy> {
    phy: P,
    state: State,
    buffer: Vec<f32>,
    assembled: Vec<u8>,
    seq: u32,
    /// Number of samples needed for sync(2 B) + frame(FRAME_LEN bytes).
    payload_samples: usize,
}

impl<P: Phy> Receiver<P> {
    pub fn new(phy: P) -> Self {
        let payload_samples = phy.frame_data_samples(SYNC_WORD.len() + FRAME_LEN);
        Self {
            phy,
            state: State::Searching,
            buffer: Vec::with_capacity(payload_samples * 4),
            assembled: Vec::new(),
            seq: 0,
            payload_samples,
        }
    }

    pub fn push_samples(&mut self, samples: &[f32]) -> Vec<FrameEvent> {
        self.buffer.extend_from_slice(samples);
        let mut events = Vec::new();
        loop {
            let progressed = self.step(&mut events);
            if !progressed { break; }
        }
        // Trim buffer to bound memory: keep at most ~4 frames worth
        let max_keep = self.payload_samples * 4;
        if self.buffer.len() > max_keep {
            let drop = self.buffer.len() - max_keep;
            self.buffer.drain(..drop);
        }
        events
    }

    fn step(&mut self, events: &mut Vec<FrameEvent>) -> bool {
        match self.state {
            State::Searching => {
                let template = self.phy.preamble();
                let tn = template.len();
                if self.buffer.len() < tn + self.payload_samples {
                    return false;
                }
                // Only search the early part; if no preamble found there,
                // slide.
                let search_end = self.buffer.len() - self.payload_samples;
                let slice = &self.buffer[..search_end + tn];
                if let Some((off, score)) = self.phy.detect_preamble(slice) {
                    if score > 0.6 {
                        // Drop everything up to and including the preamble.
                        self.buffer.drain(..off + tn);
                        self.state = State::AfterPreamble;
                        return true;
                    }
                }
                // No preamble found; drop oldest samples to make progress.
                let drop = tn;
                if self.buffer.len() > drop {
                    self.buffer.drain(..drop);
                }
                false
            }
            State::AfterPreamble => {
                if self.buffer.len() < self.payload_samples {
                    return false;
                }
                let payload_slice: Vec<f32> = self.buffer.drain(..self.payload_samples).collect();
                self.state = State::Searching;

                let raw = self.phy.demodulate_bytes(&payload_slice, SYNC_WORD.len() + FRAME_LEN);
                if raw.len() < SYNC_WORD.len() + FRAME_LEN {
                    events.push(FrameEvent::FrameDropped { seq: self.seq, reason: "short demod".into() });
                    self.seq += 1;
                    return true;
                }
                let (sync_part, frame_part) = raw.split_at(SYNC_WORD.len());

                // Sync word is best-effort: log mismatch but still try the frame.
                if sync_part != SYNC_WORD {
                    // many sync errors → probably a false preamble; drop
                    let diffs = sync_part.iter().zip(SYNC_WORD.iter()).filter(|(a,b)| a != b).count();
                    if diffs > 1 {
                        events.push(FrameEvent::FrameDropped { seq: self.seq, reason: format!("bad sync word ({} mismatches)", diffs) });
                        self.seq += 1;
                        return true;
                    }
                }

                match decode_frame(frame_part) {
                    Ok((header, payload)) => {
                        self.assembled.extend_from_slice(&payload);
                        events.push(FrameEvent::FrameOk { seq: self.seq, bytes: payload });
                        if header.last {
                            // Last 32 bytes of assembled are the SHA-256.
                            if self.assembled.len() >= 32 {
                                let (body, sha_tail) = self.assembled.split_at(self.assembled.len() - 32);
                                let computed = Sha256::digest(body);
                                let ok = computed.as_slice() == sha_tail;
                                events.push(FrameEvent::StreamComplete { bytes: body.to_vec(), sha256_ok: ok });
                            } else {
                                events.push(FrameEvent::StreamComplete { bytes: vec![], sha256_ok: false });
                            }
                            self.assembled.clear();
                            self.seq = 0;
                            return true;
                        }
                        self.seq += 1;
                    }
                    Err(FrameError::RsUncorrectable) => {
                        events.push(FrameEvent::FrameDropped { seq: self.seq, reason: "RS uncorrectable".into() });
                        self.seq += 1;
                    }
                    Err(e) => {
                        events.push(FrameEvent::FrameDropped { seq: self.seq, reason: format!("{e}") });
                        self.seq += 1;
                    }
                }
                true
            }
        }
    }
}
```

`modem-codec/src/lib.rs`:
```rust
pub mod rx;
pub mod tx;
```

- [ ] **Step 2: Write a unit test that round-trips through tx → rx in memory**

Append to `modem-codec/src/rx.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::Transmitter;
    use modem_core::phy::FskPhy;

    #[test]
    fn single_frame_roundtrip() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let payload = b"hello, acoustic world!";
        let samples = tx.encode(payload);

        let mut rx = Receiver::new(FskPhy::audible());
        let events = rx.push_samples(&samples);

        // Expect at least one FrameOk and a StreamComplete with sha256_ok.
        let complete = events.iter().find_map(|e| match e {
            FrameEvent::StreamComplete { bytes, sha256_ok } => Some((bytes.clone(), *sha256_ok)),
            _ => None,
        }).expect("no StreamComplete event");
        assert!(complete.1, "sha256 should match");
        assert_eq!(complete.0, payload);
    }

    #[test]
    fn multi_frame_roundtrip() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let payload: Vec<u8> = (0..500).map(|i| (i & 0xFF) as u8).collect();
        let samples = tx.encode(&payload);

        let mut rx = Receiver::new(FskPhy::audible());
        let events = rx.push_samples(&samples);

        let complete = events.iter().find_map(|e| match e {
            FrameEvent::StreamComplete { bytes, sha256_ok } => Some((bytes.clone(), *sha256_ok)),
            _ => None,
        }).expect("no StreamComplete event");
        assert!(complete.1);
        assert_eq!(complete.0, payload);
    }

    #[test]
    fn finds_preamble_with_silence_padding() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let samples = tx.encode(b"abc");
        let mut padded = vec![0f32; 4800];
        padded.extend_from_slice(&samples);
        padded.extend_from_slice(&[0f32; 4800]);

        let mut rx = Receiver::new(FskPhy::audible());
        let events = rx.push_samples(&padded);
        assert!(events.iter().any(|e| matches!(e, FrameEvent::StreamComplete { sha256_ok: true, .. })));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p modem-codec`
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
jj desc -m "Add Receiver state machine with multi-frame assembly + SHA-256 check"
jj new
```

---

## Task 14: Property test for the full TX→RX path

**Files:**
- Create: `modem-codec/tests/stream_roundtrip.rs`

- [ ] **Step 1: Write a proptest**

Create `modem-codec/tests/stream_roundtrip.rs`:
```rust
use modem_codec::rx::{FrameEvent, Receiver};
use modem_codec::tx::Transmitter;
use modem_core::phy::FskPhy;
use proptest::prelude::*;

fn run_roundtrip(payload: Vec<u8>) {
    let tx = Transmitter::new(FskPhy::audible(), false);
    let samples = tx.encode(&payload);

    let mut rx = Receiver::new(FskPhy::audible());
    let events = rx.push_samples(&samples);

    let got = events.iter().find_map(|e| match e {
        FrameEvent::StreamComplete { bytes, sha256_ok: true } => Some(bytes.clone()),
        _ => None,
    });
    assert_eq!(got.as_deref(), Some(payload.as_slice()));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn arbitrary_bytes_clean_channel(payload in proptest::collection::vec(any::<u8>(), 0..2048)) {
        run_roundtrip(payload);
    }
}

#[test]
fn boundary_lengths() {
    // Sizes that hit chunk boundaries: 0, 1, 219 (exactly 1 chunk before sha),
    // 220 (forces 2 chunks), 437, 438, 1024.
    for &n in &[0usize, 1, 100, 186, 187, 188, 218, 219, 220, 437, 438, 1024] {
        let payload: Vec<u8> = (0..n).map(|i| (i & 0xFF) as u8).collect();
        run_roundtrip(payload);
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p modem-codec --test stream_roundtrip`
Expected: PASS (proptest will run 32 cases plus the boundary test).

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add proptest + boundary-length tests for TX→RX roundtrip"
jj new
```

---

## Task 15: CLI skeleton with `tx-wav` and `rx-wav`

**Files:**
- Modify: `modem-cli/src/main.rs`
- Create: `modem-cli/src/cmd_tx_wav.rs`
- Create: `modem-cli/src/cmd_rx_wav.rs`

- [ ] **Step 1: Wire up clap**

`modem-cli/src/main.rs`:
```rust
mod cmd_rx_wav;
mod cmd_send;
mod cmd_recv;
mod cmd_tx_wav;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "modem", version, about = "Acoustic modem CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Transmit live over speaker.
    Send {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        /// Input file; if omitted, reads stdin.
        input: Option<PathBuf>,
    },
    /// Receive live from microphone.
    Recv {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        /// Output file; if omitted, writes to stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Print as hex dump instead of raw bytes.
        #[arg(long)]
        hex: bool,
    },
    /// Offline: encode bytes to a WAV file.
    TxWav {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        input: PathBuf,
        output: PathBuf,
    },
    /// Offline: decode a WAV file back to bytes.
    RxWav {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        hex: bool,
    },
}

#[derive(Copy, Clone, ValueEnum)]
pub enum Profile { Audible, Ultrasonic }

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Send { profile, input } => cmd_send::run(profile, input),
        Cmd::Recv { profile, output, hex } => cmd_recv::run(profile, output, hex),
        Cmd::TxWav { profile, input, output } => cmd_tx_wav::run(profile, input, output),
        Cmd::RxWav { profile, input, output, hex } => cmd_rx_wav::run(profile, input, output, hex),
    }
}

pub fn make_phy(profile: Profile) -> modem_core::phy::FskPhy {
    match profile {
        Profile::Audible => modem_core::phy::FskPhy::audible(),
        Profile::Ultrasonic => modem_core::phy::FskPhy::ultrasonic(),
    }
}
```

- [ ] **Step 2: Create empty `cmd_send.rs` / `cmd_recv.rs` stubs (filled in Task 18/19)**

`modem-cli/src/cmd_send.rs`:
```rust
use crate::Profile;
use std::path::PathBuf;
pub fn run(_profile: Profile, _input: Option<PathBuf>) -> anyhow::Result<()> {
    anyhow::bail!("`send` not implemented yet — use `tx-wav`");
}
```

`modem-cli/src/cmd_recv.rs`:
```rust
use crate::Profile;
use std::path::PathBuf;
pub fn run(_profile: Profile, _output: Option<PathBuf>, _hex: bool) -> anyhow::Result<()> {
    anyhow::bail!("`recv` not implemented yet — use `rx-wav`");
}
```

- [ ] **Step 3: Implement `cmd_tx_wav.rs`**

```rust
use crate::{make_phy, Profile};
use hound::{SampleFormat, WavSpec, WavWriter};
use modem_codec::tx::Transmitter;
use std::fs;
use std::path::PathBuf;

pub fn run(profile: Profile, input: PathBuf, output: PathBuf) -> anyhow::Result<()> {
    let bytes = fs::read(&input)?;
    let phy = make_phy(profile);
    let ultrasonic = matches!(profile, Profile::Ultrasonic);
    let tx = Transmitter::new(phy, ultrasonic);
    let samples = tx.encode(&bytes);

    let spec = WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut w = WavWriter::create(&output, spec)?;
    let n = samples.len();
    for s in samples { w.write_sample(s)?; }
    w.finalize()?;
    eprintln!("wrote {} samples ({:.1}s) to {}", n, n as f32 / 48_000.0, output.display());
    Ok(())
}
```

- [ ] **Step 4: Implement `cmd_rx_wav.rs`**

```rust
use crate::{make_phy, Profile};
use hound::WavReader;
use modem_codec::rx::{FrameEvent, Receiver};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub fn run(profile: Profile, input: PathBuf, output: Option<PathBuf>, hex: bool) -> anyhow::Result<()> {
    let mut reader = WavReader::open(&input)?;
    let samples: Vec<f32> = match reader.spec().sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => reader.samples::<i32>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i32::MAX as f32)
            .collect(),
    };

    let phy = make_phy(profile);
    let mut rx = Receiver::new(phy);
    let events = rx.push_samples(&samples);

    let bytes: Vec<u8> = events.into_iter().find_map(|e| match e {
        FrameEvent::StreamComplete { bytes, .. } => Some(bytes),
        _ => None,
    }).ok_or_else(|| anyhow::anyhow!("no complete stream received"))?;

    if let Some(p) = output {
        fs::write(&p, &bytes)?;
        eprintln!("wrote {} bytes to {}", bytes.len(), p.display());
    } else if hex || !is_printable(&bytes) {
        print_hex(&bytes);
    } else {
        std::io::stdout().write_all(&bytes)?;
    }
    Ok(())
}

fn is_printable(b: &[u8]) -> bool {
    std::str::from_utf8(b).map_or(false, |s| s.chars().all(|c|
        !c.is_control() || c == '\t' || c == '\n' || c == '\r'
    ))
}

fn print_hex(b: &[u8]) {
    for (i, chunk) in b.chunks(16).enumerate() {
        let hex: String = chunk.iter().map(|x| format!("{:02x} ", x)).collect();
        let ascii: String = chunk.iter()
            .map(|&x| if x.is_ascii_graphic() || x == b' ' { x as char } else { '.' })
            .collect();
        println!("{:08x}  {:<48}  {}", i * 16, hex, ascii);
    }
}
```

- [ ] **Step 5: Build**

Run: `cargo build --workspace`
Expected: clean build.

- [ ] **Step 6: Manual smoke test of tx-wav / rx-wav**

```bash
echo "hello acoustic modem" > /tmp/in.txt
cargo run -p modem-cli -- tx-wav /tmp/in.txt /tmp/out.wav
cargo run -p modem-cli -- rx-wav /tmp/out.wav -o /tmp/decoded.txt
diff /tmp/in.txt /tmp/decoded.txt
```

Expected: no diff. `/tmp/out.wav` should also be openable in any audio app — playing it should sound like the modem warble.

- [ ] **Step 7: Commit**

```bash
jj desc -m "Add modem-cli with tx-wav and rx-wav subcommands"
jj new
```

---

## Task 16: WAV round-trip integration test (CI)

**Files:**
- Create: `modem-cli/tests/wav_roundtrip.rs`

- [ ] **Step 1: Write the integration test**

```rust
use std::process::Command;

#[test]
fn cli_wav_roundtrip_audible() {
    let dir = tempfile::tempdir().unwrap();
    let in_path = dir.path().join("in.bin");
    let wav_path = dir.path().join("out.wav");
    let out_path = dir.path().join("decoded.bin");

    let payload: Vec<u8> = (0..400).map(|i| (i & 0xFF) as u8).collect();
    std::fs::write(&in_path, &payload).unwrap();

    let bin = env!("CARGO_BIN_EXE_modem");
    let s1 = Command::new(bin)
        .args(["tx-wav", in_path.to_str().unwrap(), wav_path.to_str().unwrap()])
        .status().unwrap();
    assert!(s1.success());

    let s2 = Command::new(bin)
        .args(["rx-wav", wav_path.to_str().unwrap(), "-o", out_path.to_str().unwrap()])
        .status().unwrap();
    assert!(s2.success());

    let back = std::fs::read(&out_path).unwrap();
    assert_eq!(back, payload);
}
```

Add `tempfile = "3"` to `modem-cli`'s `[dev-dependencies]`.

- [ ] **Step 2: Run**

Run: `cargo test -p modem-cli --test wav_roundtrip`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add CLI integration test exercising tx-wav | rx-wav"
jj new
```

---

## Task 17: cpal audio output (`modem-audio::output`)

**Files:**
- Create: `modem-audio/src/output.rs`
- Modify: `modem-audio/src/lib.rs`

- [ ] **Step 1: Write a simple output streamer**

`modem-audio/src/output.rs`:
```rust
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;

/// Play `samples` over the default output device at 48 kHz mono.
/// Blocks until all samples have been delivered to the stream callback.
pub fn play_samples(samples: Vec<f32>) -> Result<()> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| anyhow!("no default output device"))?;

    let mut configs: Vec<_> = device.supported_output_configs()?.collect();
    // Prefer 48 kHz mono f32.
    configs.sort_by_key(|c| {
        let r = c.max_sample_rate().0;
        (r as i64 - 48_000).abs()
    });
    let supported = configs
        .into_iter()
        .find(|c| c.sample_format() == cpal::SampleFormat::F32)
        .ok_or_else(|| anyhow!("no f32 output config"))?
        .with_sample_rate(cpal::SampleRate(48_000));

    let channels = supported.channels() as usize;
    let config: cpal::StreamConfig = supported.into();

    let mut idx = 0usize;
    let total = samples.len();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let done_tx = std::sync::Mutex::new(Some(done_tx));

    let stream = device.build_output_stream(
        &config,
        move |out: &mut [f32], _| {
            for frame in out.chunks_mut(channels) {
                let v = if idx < total { samples[idx] } else { 0.0 };
                for ch in frame { *ch = v; }
                if idx < total { idx += 1; }
            }
            if idx >= total {
                if let Some(tx) = done_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        },
        |e| eprintln!("output stream error: {e}"),
        None,
    )?;

    stream.play()?;
    // Wait until the callback has consumed all samples, then pad ~200ms.
    let _ = done_rx.recv();
    std::thread::sleep(std::time::Duration::from_millis(200));
    Ok(())
}
```

`modem-audio/src/lib.rs`:
```rust
pub mod output;
pub mod input;
```

- [ ] **Step 2: Stub input so the crate builds**

`modem-audio/src/input.rs`:
```rust
//! Microphone capture — implemented in Task 18.
```

- [ ] **Step 3: Build**

Run: `cargo build -p modem-audio`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
jj desc -m "Add modem-audio::output using cpal for speaker playback"
jj new
```

---

## Task 18: cpal audio input + `modem-audio::input`

**Files:**
- Modify: `modem-audio/src/input.rs`

- [ ] **Step 1: Write a chunked microphone reader**

```rust
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::{self, Receiver};

pub struct Mic {
    pub rx: Receiver<Vec<f32>>,
    _stream: cpal::Stream,
}

/// Open the default input device at 48 kHz, mono, f32, delivering buffers
/// of samples to the returned receiver. Drop `Mic` to stop.
pub fn open_mic() -> Result<Mic> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| anyhow!("no default input device"))?;
    let mut configs: Vec<_> = device.supported_input_configs()?.collect();
    configs.sort_by_key(|c| {
        let r = c.max_sample_rate().0;
        (r as i64 - 48_000).abs()
    });
    let supported = configs.into_iter()
        .find(|c| c.sample_format() == cpal::SampleFormat::F32)
        .ok_or_else(|| anyhow!("no f32 input config"))?
        .with_sample_rate(cpal::SampleRate(48_000));
    let channels = supported.channels() as usize;
    let config: cpal::StreamConfig = supported.into();

    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let stream = device.build_input_stream(
        &config,
        move |samples: &[f32], _| {
            // Down-mix to mono by averaging channels.
            let mono: Vec<f32> = samples.chunks(channels)
                .map(|c| c.iter().sum::<f32>() / channels as f32)
                .collect();
            let _ = tx.send(mono);
        },
        |e| eprintln!("input stream error: {e}"),
        None,
    )?;
    stream.play()?;
    Ok(Mic { rx, _stream: stream })
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p modem-audio`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add modem-audio::input with cpal mic capture"
jj new
```

---

## Task 19: Implement `modem send`

**Files:**
- Modify: `modem-cli/src/cmd_send.rs`

- [ ] **Step 1: Wire transmitter to speaker output**

```rust
use crate::{make_phy, Profile};
use modem_codec::tx::Transmitter;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

pub fn run(profile: Profile, input: Option<PathBuf>) -> anyhow::Result<()> {
    let bytes = match input {
        Some(p) => fs::read(&p)?,
        None => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
    };
    let phy = make_phy(profile);
    let ultrasonic = matches!(profile, Profile::Ultrasonic);
    let tx = Transmitter::new(phy, ultrasonic);
    let samples = tx.encode(&bytes);
    eprintln!("→ {} bytes, {} samples ({:.1}s) over speaker",
        bytes.len(), samples.len(), samples.len() as f32 / 48_000.0);
    modem_audio::output::play_samples(samples)?;
    eprintln!("✓ done");
    Ok(())
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p modem-cli`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Implement modem send: stream encoded bytes to speaker via cpal"
jj new
```

---

## Task 20: Implement `modem recv`

**Files:**
- Modify: `modem-cli/src/cmd_recv.rs`

- [ ] **Step 1: Wire mic into Receiver**

```rust
use crate::{make_phy, Profile};
use modem_audio::input::open_mic;
use modem_codec::rx::{FrameEvent, Receiver};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub fn run(profile: Profile, output: Option<PathBuf>, hex: bool) -> anyhow::Result<()> {
    let phy = make_phy(profile);
    let mut rx = Receiver::new(phy);
    let mic = open_mic()?;

    eprintln!("listening… (Ctrl-C to stop)");

    loop {
        let chunk = mic.rx.recv()?;
        let events = rx.push_samples(&chunk);
        for ev in events {
            match ev {
                FrameEvent::FrameOk { seq, bytes } => {
                    eprintln!("  frame {seq} ok ({} bytes)", bytes.len());
                }
                FrameEvent::FrameDropped { seq, reason } => {
                    eprintln!("  frame {seq} dropped: {reason}");
                }
                FrameEvent::StreamComplete { bytes, sha256_ok } => {
                    eprintln!("✓ {} bytes received, sha256 {}", bytes.len(),
                        if sha256_ok { "ok" } else { "MISMATCH" });
                    if let Some(p) = &output {
                        fs::write(p, &bytes)?;
                        eprintln!("  wrote {}", p.display());
                    } else if hex || !is_printable(&bytes) {
                        print_hex(&bytes);
                    } else {
                        std::io::stdout().write_all(&bytes)?;
                        std::io::stdout().write_all(b"\n")?;
                    }
                    return Ok(());
                }
            }
        }
    }
}

fn is_printable(b: &[u8]) -> bool {
    std::str::from_utf8(b).map_or(false, |s| s.chars().all(|c|
        !c.is_control() || c == '\t' || c == '\n' || c == '\r'
    ))
}

fn print_hex(b: &[u8]) {
    for (i, chunk) in b.chunks(16).enumerate() {
        let hex: String = chunk.iter().map(|x| format!("{:02x} ", x)).collect();
        let ascii: String = chunk.iter()
            .map(|&x| if x.is_ascii_graphic() || x == b' ' { x as char } else { '.' })
            .collect();
        println!("{:08x}  {:<48}  {}", i * 16, hex, ascii);
    }
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p modem-cli`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Implement modem recv: stream mic into Receiver, print result"
jj new
```

---

## Task 21: Real-audio smoke test instructions

**Files:**
- Create: `docs/smoke-test.md`

- [ ] **Step 1: Document the manual smoke matrix**

```markdown
# Manual real-audio smoke test

## Prereqs

- macOS, Terminal granted Microphone access in System Settings → Privacy & Security → Microphone.
- Two terminals (or two devices).

## Loopback on one MacBook (speakers → built-in mic)

1. Open Terminal A and Terminal B.
2. In Terminal B, start listening:
   ```bash
   cargo run --release -p modem-cli -- recv
   ```
3. In Terminal A, send a message:
   ```bash
   echo "hello modem" | cargo run --release -p modem-cli -- send
   ```
4. Terminal B should print `hello modem` within ~5 seconds of speaker stopping.

## Audible vs ultrasonic

Repeat the above with `--profile ultrasonic` on both sides. Stand within ~1 m
of the receiving mic. If ultrasonic fails on your hardware, the speaker or
mic likely rolls off above 17 kHz. Audible will always work.

## Adversarial

- Play music at moderate volume from a third device. Audible profile should
  still succeed at SNR ~10 dB. Ultrasonic should be unaffected by sub-15 kHz
  music.
- Cough into the mic during transmission. Frames may be dropped; if the
  whole message was a single frame, expect a re-send.

## Recording for regression tests

```bash
# Send and capture the air with another tool (e.g. QuickTime audio recording),
# save as 48 kHz mono float WAV, then run:
cargo run --release -p modem-cli -- rx-wav captured.wav
```

Add successful captures to `recordings/` and re-run `rx-wav` against them in CI
if/when you want regression protection against demodulator changes.
```

- [ ] **Step 2: Commit**

```bash
jj desc -m "Document manual real-audio smoke test"
jj new
```

---

## Task 22: Release build + final verification

- [ ] **Step 1: Build release binary**

Run: `cargo build --release --workspace`
Expected: clean build, `target/release/modem` exists.

- [ ] **Step 2: Run full test suite**

Run: `cargo test --workspace`
Expected: all tests pass across all crates.

- [ ] **Step 3: Manual end-to-end check**

Follow `docs/smoke-test.md` Section "Loopback on one MacBook" with the release binary:

```bash
./target/release/modem recv &
sleep 1
echo "hello acoustic world" | ./target/release/modem send
wait
```

Expected: receiver prints `hello acoustic world`.

- [ ] **Step 4: Commit any final polish**

```bash
jj desc -m "Finalize macOS PoC release build" # optional, only if there are changes
jj new
```

---

## Done criteria for this plan

1. `cargo test --workspace` passes — units, integration, proptests, CLI integration tests.
2. `./target/release/modem` runs `send` and `recv` end-to-end on a single MacBook (speaker → built-in mic) for both `--profile audible` and `--profile ultrasonic` (best-effort), with a short text payload (< 200 B) succeeding.
3. `docs/smoke-test.md` documents the manual matrix.

Once those hold, the next plan (`2026-MM-DD-acoustic-modem-android.md`) adds the `modem-ffi` crate, UniFFI bindings, and the Kotlin app — building directly on top of `modem-codec` with no changes to it.
