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

> The CLI auto-enables a live ratatui dashboard (tone bars + frame timeline
> + status) when stdout is a TTY. Pass `--plain` to revert to the
> line-by-line text output (matches the historical behaviour and is
> required for scripts that grep stdout).

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
