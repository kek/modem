# Capture corpus

WAV recordings of real over-the-air transmissions plus a manifest describing what each one was supposed to contain. Consumed by `modem rank` to compare DSP variants against a fixed reference set without re-recording.

## Adding a capture

1. Record the audio. The receiver hardware should save to a 48 kHz mono float WAV. Two convenient ways:
   - From the Mac side: launch QuickTime audio recording, transmit from phone, save the result, then trim to start ~200 ms before the preamble chirp.
   - From the Android side: TBD — the Android app needs a "save received audio" debug switch (not yet implemented).
2. Drop the WAV into this directory.
3. Append an entry to `manifest.toml`:
   ```toml
   [[capture]]
   file = "android-to-mac-001.wav"
   expected = "hello from android"      # plain UTF-8 payload
   # OR
   # expected_hex = "deadbeef"          # arbitrary bytes
   direction = "android-to-mac"         # free-form label, surfaced in output
   profile = "audible"                  # "audible" | "ultrasonic"
   ```

## Running

```bash
./target/release/modem rank captures/
```

Emits one structured line per (capture × variant) and a per-variant summary. Easy to consume from a shell or an agent:

```bash
./target/release/modem rank captures/ | grep '^SUMMARY' | sort -t= -k3 -n
```

## What's in the box

- `manifest.toml` — capture catalog (always commit this).
- `*.wav` — actual audio files. Large; consider Git LFS if the corpus grows.

Empty manifest is OK — `rank` will just print no per-cell lines and no summaries.
