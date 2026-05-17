//! Interactive chat: type a line, press Enter to transmit. Mic input is
//! decoded concurrently so received messages appear in the log. Shares the
//! tone-bar + frame-timeline visualization with `recv`/`send`.

use crate::{make_phy, Profile};
use modem_audio::input::open_mic;
use modem_codec::rx::{FrameEvent, Receiver};
use modem_codec::tx::Transmitter;
use modem_core::frame::{FRAME_LEN, HEADER_LEN, MAX_PAYLOAD};
use modem_core::fsk::{goertzel_bank, DspVariants, FskConfig};
use modem_core::phy::{FskPhy, Phy};
use modem_core::preamble::SYNC_WORD;
use std::io::stdout;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Position},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

const TONE_EMA_ALPHA: f32 = 0.4;
const DB_FLOOR: f32 = -80.0;
const DB_CEIL: f32 = 0.0;
const MAX_LOG: usize = 200;
const MAX_TIMELINE: usize = 64;

#[derive(Clone)]
enum LogEntry {
    Sent(String),
    Recv { text: String, sha_ok: bool },
    Dropped { seq: u32, reason: String },
    Info(String),
}

struct TxInFlight {
    samples: Vec<f32>,
    cursor: usize,
    start: Instant,
    total_sec: f32,
    bytes: usize,
    done_rx: mpsc::Receiver<()>,
    /// Index into the chat log of the matching `Sent` entry, so the renderer
    /// can highlight characters up to the current playback progress.
    log_pos: usize,
    /// Segmented breakdown of the encoded buffer for the structure panel.
    segments: Vec<Segment>,
    /// True once playback has finished. The panel stays visible so the user
    /// can inspect the breakdown; the next Enter will replace this state.
    done: bool,
}

#[derive(Clone)]
struct Segment {
    start_sample: usize,
    end_sample: usize,
    kind: SegKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SegKind {
    Preamble,
    Sync,
    Header,
    Payload,
    Padding,
    RsParity,
    Crc,
}

impl SegKind {
    fn color(self) -> Color {
        match self {
            SegKind::Preamble => Color::Magenta,
            SegKind::Sync => Color::Cyan,
            SegKind::Header => Color::Blue,
            SegKind::Payload => Color::Green,
            SegKind::Padding => Color::Gray,
            SegKind::RsParity => Color::Yellow,
            SegKind::Crc => Color::Red,
        }
    }
}

pub fn run(profile: Profile, variants: DspVariants) -> anyhow::Result<()> {
    let cfg = match profile {
        Profile::Audible => FskConfig::audible(),
        Profile::Ultrasonic => FskConfig::ultrasonic(),
    };
    let freqs: Vec<f32> = cfg.tone_freqs.to_vec();
    let sample_rate = cfg.sample_rate;

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let result = run_loop(profile, variants, freqs, sample_rate);
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);
    result
}

fn run_loop(
    profile: Profile,
    variants: DspVariants,
    freqs: Vec<f32>,
    sample_rate: u32,
) -> anyhow::Result<()> {
    let mut term = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mic = open_mic()?;
    let phy_rx = make_phy(profile, variants);
    let mut rx = Receiver::new(phy_rx);
    let phy_tx = make_phy(profile, variants);
    let ultrasonic = matches!(profile, Profile::Ultrasonic);
    let tx = Transmitter::new(phy_tx, ultrasonic);

    let mut tones_db: Vec<f32> = vec![DB_FLOOR; freqs.len()];
    let mut rms = 0.0f32;
    let mut timeline: Vec<(u32, bool)> = Vec::new(); // (seq, ok)
    let mut total_ok = 0usize;
    let mut total_dropped = 0usize;
    let mut log: Vec<LogEntry> = Vec::new();
    log.push(LogEntry::Info(format!(
        "interactive chat on {} · Enter to send · Esc/Ctrl-D to quit",
        if ultrasonic { "ultrasonic" } else { "audible" }
    )));

    let mut input = String::new();
    let mut tx_state: Option<TxInFlight> = None;
    let last_draw = Instant::now();
    let mut next_draw = last_draw;

    loop {
        // 1) Drain mic into RX decoder + tone-bar feeder (skip tone feed
        //    during TX so the bars don't pick up our own outgoing signal).
        let tx_active = tx_state.as_ref().map(|t| !t.done).unwrap_or(false);
        while let Ok(chunk) = mic.rx.try_recv() {
            if !tx_active {
                let events = rx.push_samples(&chunk);
                for ev in events {
                    match ev {
                        FrameEvent::FrameOk { seq, .. } => {
                            push_timeline(&mut timeline, seq, true);
                            total_ok += 1;
                        }
                        FrameEvent::FrameDropped { seq, reason } => {
                            push_timeline(&mut timeline, seq, false);
                            total_dropped += 1;
                            push_log(&mut log, LogEntry::Dropped { seq, reason });
                        }
                        FrameEvent::StreamComplete { bytes, sha256_ok } => {
                            let text = match std::str::from_utf8(&bytes) {
                                Ok(s) => s.to_string(),
                                Err(_) => format!("<{} bytes>", bytes.len()),
                            };
                            push_log(&mut log, LogEntry::Recv { text, sha_ok: sha256_ok });
                        }
                    }
                }
                update_tones(&mut tones_db, &mut rms, &chunk, &freqs, sample_rate);
            }
        }

        // 2) Advance TX feeder if a transmission is in flight. Pace the
        //    cursor to wall-clock so the tone bars and char highlighter stay
        //    roughly in sync with what's coming out of the speaker.
        if let Some(t) = tx_state.as_mut() {
            if !t.done {
                let elapsed = t.start.elapsed().as_secs_f32();
                let want = ((elapsed / t.total_sec.max(1e-6)) * t.samples.len() as f32) as usize;
                let want = want.min(t.samples.len());
                if want > t.cursor {
                    update_tones(&mut tones_db, &mut rms, &t.samples[t.cursor..want], &freqs, sample_rate);
                    t.cursor = want;
                }
                if t.done_rx.try_recv().is_ok() {
                    // Playback finished: light up the entire bar, mark done,
                    // and leave the panel visible for inspection. Reset the
                    // RX decoder so residual mic buffer (echo of our own
                    // outgoing) doesn't get misinterpreted as a fresh frame.
                    t.cursor = t.samples.len();
                    t.done = true;
                    rx = Receiver::new(make_phy(profile, variants));
                }
            }
        }

        // 3) Handle keyboard input (non-blocking).
        if event::poll(Duration::from_millis(0))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        KeyCode::Esc => break,
                        KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => break,
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => break,
                        KeyCode::Enter => {
                            let can_send = tx_state.as_ref().map(|t| t.done).unwrap_or(true);
                            if !input.is_empty() && can_send {
                                let bytes = input.as_bytes().to_vec();
                                let samples = tx.encode(&bytes);
                                let total_sec = samples.len() as f32 / sample_rate as f32;
                                let segments = compute_segments(&bytes, profile, variants);
                                push_log(&mut log, LogEntry::Sent(input.clone()));
                                let log_pos = log.len() - 1;
                                let samples_for_play = samples.clone();
                                let (done_tx, done_rx) = mpsc::channel::<()>();
                                thread::spawn(move || {
                                    let _ = modem_audio::output::play_samples(samples_for_play);
                                    let _ = done_tx.send(());
                                });
                                tx_state = Some(TxInFlight {
                                    samples,
                                    cursor: 0,
                                    start: Instant::now(),
                                    total_sec,
                                    bytes: bytes.len(),
                                    done_rx,
                                    log_pos,
                                    segments,
                                    done: false,
                                });
                                input.clear();
                            }
                        }
                        KeyCode::Backspace => { input.pop(); }
                        KeyCode::Char(c) => {
                            if !c.is_control() {
                                input.push(c);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // 4) Redraw at ~30 Hz.
        let now = Instant::now();
        if now >= next_draw {
            next_draw = now + Duration::from_millis(33);
            let (tx_bytes, tx_total, tx_elapsed, tx_done) = match tx_state.as_ref() {
                Some(t) => {
                    let elapsed = t.start.elapsed().as_secs_f32().min(t.total_sec);
                    (Some(t.bytes), Some(t.total_sec), Some(elapsed), t.done)
                }
                None => (None, None, None, false),
            };
            let tx_snapshot = tx_state.as_ref().map(|t| {
                (
                    t.cursor,
                    t.samples.len(),
                    t.segments.clone(),
                    t.done,
                )
            });
            term.draw(|f| {
                let area = f.area();
                let struct_h: u16 = if tx_snapshot.is_some() { 4 } else { 0 };
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(11),
                        Constraint::Length(struct_h),
                        Constraint::Length(3),
                        Constraint::Min(3),
                        Constraint::Length(3),
                    ])
                    .split(area);

                // Tone bars.
                let mut bar_lines = String::new();
                for (db, hz) in tones_db.iter().zip(freqs.iter()) {
                    let level = ((*db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0);
                    bar_lines.push_str(&format!("{:>5.2}k  {}\n", hz / 1000.0, blocks_for(level)));
                }
                let title = match (tx_bytes, tx_total, tx_elapsed) {
                    (Some(b), Some(t), Some(e)) if !tx_done => format!(
                        " tones · rms {:.3} · sending {} B {:.1}/{:.1} s ",
                        rms, b, e, t,
                    ),
                    (Some(b), Some(t), _) if tx_done => format!(
                        " tones · rms {:.3} · sent {} B in {:.1} s ",
                        rms, b, t,
                    ),
                    _ => format!(" tones · rms {:.3} ", rms),
                };
                f.render_widget(
                    Paragraph::new(bar_lines).block(Block::default().borders(Borders::ALL).title(title)),
                    layout[0],
                );

                // Frame structure panel (visible during and after TX).
                if let Some((cursor, total, segments, done)) = tx_snapshot.as_ref() {
                    let inner_w = layout[1].width.saturating_sub(2) as usize;
                    let (legend, bar) = render_structure(*cursor, *total, segments, inner_w);
                    let title = if *done { " frame structure (sent) " } else { " frame structure " };
                    f.render_widget(
                        Paragraph::new(vec![legend, bar]).block(
                            Block::default().borders(Borders::ALL).title(title)
                        ),
                        layout[1],
                    );
                }

                // Frame timeline chips.
                let chips: Vec<Span> = timeline.iter().flat_map(|(seq, ok)| {
                    let (label, color) = if *ok {
                        (format!("[ok {seq}]"), Color::Green)
                    } else {
                        (format!("[DROP {seq}]"), Color::Red)
                    };
                    vec![Span::styled(label, Style::default().fg(color)), Span::raw(" ")]
                }).collect();
                f.render_widget(
                    Paragraph::new(Line::from(chips)).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" frames · {total_ok} ok · {total_dropped} dropped "))
                    ),
                    layout[2],
                );

                // Message log (most recent at bottom, scrolling).
                let log_height = layout[3].height.saturating_sub(2) as usize;
                let log_start = log.len().saturating_sub(log_height);
                // Animate the in-flight Sent line's character highlight only
                // while playback is still progressing. Once done, leave the
                // line in its normal style.
                let in_flight = tx_state.as_ref().and_then(|t| {
                    if t.done {
                        None
                    } else {
                        let progress = (t.cursor as f32 / t.samples.len().max(1) as f32).clamp(0.0, 1.0);
                        Some((t.log_pos, progress))
                    }
                });
                let lines: Vec<Line> = log
                    .iter()
                    .enumerate()
                    .skip(log_start)
                    .map(|(idx, e)| match e {
                        LogEntry::Sent(t) => {
                            let arrow = Span::styled("→ ", Style::default().fg(Color::Cyan));
                            match in_flight {
                                Some((pos, progress)) if pos == idx => {
                                    let chars: Vec<char> = t.chars().collect();
                                    let n = ((chars.len() as f32) * progress).round() as usize;
                                    let n = n.min(chars.len());
                                    let lit: String = chars[..n].iter().collect();
                                    let dim: String = chars[n..].iter().collect();
                                    Line::from(vec![
                                        arrow,
                                        Span::styled(
                                            lit,
                                            Style::default().fg(Color::Black).bg(Color::Cyan),
                                        ),
                                        Span::styled(dim, Style::default().fg(Color::DarkGray)),
                                    ])
                                }
                                _ => Line::from(vec![arrow, Span::raw(t.clone())]),
                            }
                        }
                        LogEntry::Recv { text, sha_ok } => {
                            let mark = if *sha_ok { "← " } else { "←! " };
                            let color = if *sha_ok { Color::Green } else { Color::Yellow };
                            Line::from(vec![
                                Span::styled(mark, Style::default().fg(color)),
                                Span::raw(text.clone()),
                            ])
                        }
                        LogEntry::Dropped { seq, reason } => Line::from(vec![
                            Span::styled(
                                format!("✗ frame {seq} dropped: "),
                                Style::default().fg(Color::Red),
                            ),
                            Span::raw(reason.clone()),
                        ]),
                        LogEntry::Info(t) => Line::from(Span::styled(
                            t.clone(),
                            Style::default().fg(Color::DarkGray),
                        )),
                    })
                    .collect();
                f.render_widget(
                    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" chat ")),
                    layout[3],
                );

                // Input prompt.
                let prompt = format!("> {input}");
                f.render_widget(
                    Paragraph::new(prompt.clone()).block(Block::default().borders(Borders::ALL).title(" type ")),
                    layout[4],
                );
                // Place the terminal cursor after the prompt + input text.
                let cursor_x = layout[4].x + 1 + (prompt.chars().count() as u16);
                let cursor_y = layout[4].y + 1;
                f.set_cursor_position(Position::new(cursor_x, cursor_y));
            })?;
        }

        thread::sleep(Duration::from_millis(8));
    }

    Ok(())
}

fn push_log(log: &mut Vec<LogEntry>, e: LogEntry) {
    if log.len() >= MAX_LOG {
        log.remove(0);
    }
    log.push(e);
}

fn push_timeline(timeline: &mut Vec<(u32, bool)>, seq: u32, ok: bool) {
    if timeline.len() >= MAX_TIMELINE {
        timeline.remove(0);
    }
    timeline.push((seq, ok));
}

fn update_tones(
    tones_db: &mut [f32],
    rms: &mut f32,
    samples: &[f32],
    freqs: &[f32],
    sample_rate: u32,
) {
    if samples.is_empty() { return; }
    let mags = goertzel_bank(samples, freqs, sample_rate);
    let norm = (samples.len().max(1) as f32).powi(2);
    let mut sum_sq = 0.0f64;
    for x in samples { sum_sq += (*x as f64) * (*x as f64); }
    *rms = (sum_sq / samples.len() as f64).sqrt() as f32;
    for (i, m) in mags.iter().enumerate() {
        let v = m / norm;
        let db = if v > 0.0 { 10.0 * v.log10() } else { DB_FLOOR };
        tones_db[i] += TONE_EMA_ALPHA * (db - tones_db[i]);
    }
}

/// Build the segment list for a future transmission of `payload`. Mirrors
/// `Transmitter::encode` so the cursor in the structure panel lines up with
/// what's actually coming out of the speaker.
fn compute_segments(payload: &[u8], profile: Profile, variants: DspVariants) -> Vec<Segment> {
    // Recreate the exact byte layout the transmitter produces: SHA-256 is
    // appended to the user payload, then the result is chunked into
    // MAX_PAYLOAD-sized frame payloads.
    use sha2::{Digest, Sha256};
    let sha = Sha256::digest(payload);
    let mut full = Vec::with_capacity(payload.len() + 32);
    full.extend_from_slice(payload);
    full.extend_from_slice(&sha);
    let chunks: Vec<&[u8]> = full.chunks(MAX_PAYLOAD).collect();

    let phy = make_phy(profile, variants);
    let preamble_len = <FskPhy as Phy>::preamble(&phy).len();
    let frame_data_samples = phy.frame_data_samples(SYNC_WORD.len() + FRAME_LEN);
    // Within frame_data_samples (covers SYNC + FRAME), proportional offsets
    // by byte position.
    let total_bytes_in_frame = SYNC_WORD.len() + FRAME_LEN;
    let byte_to_sample = |b: usize| -> usize {
        // Proportional: bytes are packed into 3-bit symbols then modulated,
        // but for visualization, linear interpolation is close enough.
        ((b as f32 / total_bytes_in_frame as f32) * frame_data_samples as f32) as usize
    };

    let mut out = Vec::new();
    let mut cursor = 0usize;
    for chunk in chunks.iter() {
        // Preamble
        out.push(Segment {
            start_sample: cursor,
            end_sample: cursor + preamble_len,
            kind: SegKind::Preamble,
        });
        cursor += preamble_len;

        // Within the modulated SYNC+FRAME block.
        // Byte layout (offsets from start of SYNC):
        //   0..2:                       SYNC
        //   2..2+HEADER_LEN(4):         frame header
        //   2+4..2+4+chunk_len:         user payload (within RS data block)
        //   2+4+chunk_len..2+4+219:     zero padding
        //   2+223..2+255:               RS parity (32 bytes)
        //   2+255..2+259:               CRC32 (4 bytes)
        let frame_start = cursor;
        let s_sync = 0;
        let s_header = SYNC_WORD.len();
        let s_payload = s_header + HEADER_LEN;
        let s_padding = s_payload + chunk.len();
        let s_crc = SYNC_WORD.len() + 255; // RS_DATA + RS_PARITY
        let s_end = SYNC_WORD.len() + FRAME_LEN; // 261

        // Sync
        out.push(Segment {
            start_sample: frame_start + byte_to_sample(s_sync),
            end_sample: frame_start + byte_to_sample(s_header),
            kind: SegKind::Sync,
        });
        // Header
        out.push(Segment {
            start_sample: frame_start + byte_to_sample(s_header),
            end_sample: frame_start + byte_to_sample(s_payload),
            kind: SegKind::Header,
        });
        // Payload (only if non-empty)
        if s_padding > s_payload {
            out.push(Segment {
                start_sample: frame_start + byte_to_sample(s_payload),
                end_sample: frame_start + byte_to_sample(s_padding),
                kind: SegKind::Payload,
            });
        }
        let s_rs = SYNC_WORD.len() + 223; // RS_DATA = 223
        // Zero padding (only if chunk shorter than MAX_PAYLOAD)
        if s_rs > s_padding {
            out.push(Segment {
                start_sample: frame_start + byte_to_sample(s_padding),
                end_sample: frame_start + byte_to_sample(s_rs),
                kind: SegKind::Padding,
            });
        }
        // RS parity (32 bytes after the data block)
        out.push(Segment {
            start_sample: frame_start + byte_to_sample(s_rs),
            end_sample: frame_start + byte_to_sample(s_crc),
            kind: SegKind::RsParity,
        });
        // CRC
        out.push(Segment {
            start_sample: frame_start + byte_to_sample(s_crc),
            end_sample: frame_start + byte_to_sample(s_end),
            kind: SegKind::Crc,
        });

        cursor += frame_data_samples;
    }
    out
}

/// Render the structure panel as (legend, bar) lines, sized to `width` cells.
fn render_structure<'a>(
    cursor: usize,
    total: usize,
    segments: &[Segment],
    width: usize,
) -> (Line<'a>, Line<'a>) {
    let total = total.max(1);
    // Legend (single line of color keys).
    let legend = Line::from(vec![
        Span::styled("P", Style::default().fg(SegKind::Preamble.color())),
        Span::raw("=pream "),
        Span::styled("S", Style::default().fg(SegKind::Sync.color())),
        Span::raw("=sync "),
        Span::styled("H", Style::default().fg(SegKind::Header.color())),
        Span::raw("=hdr "),
        Span::styled("D", Style::default().fg(SegKind::Payload.color())),
        Span::raw("=data "),
        Span::styled("░", Style::default().fg(SegKind::Padding.color())),
        Span::raw("=pad "),
        Span::styled("R", Style::default().fg(SegKind::RsParity.color())),
        Span::raw("=rs "),
        Span::styled("C", Style::default().fg(SegKind::Crc.color())),
        Span::raw("=crc"),
    ]);

    // Bar: one cell per `total/width` samples; color = covering segment;
    // glyph = solid if cursor has reached the cell, dim otherwise.
    let mut spans: Vec<Span<'a>> = Vec::with_capacity(width);
    for cell in 0..width {
        let cell_sample = ((cell as f32 / width as f32) * total as f32) as usize;
        let lit = cell_sample <= cursor;
        let kind = segment_at(segments, cell_sample);
        let color = kind.map(|k| k.color()).unwrap_or(Color::DarkGray);
        let glyph = if lit { "█" } else { "░" };
        let style = if lit {
            Style::default().fg(color)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(glyph.to_string(), style));
    }
    (legend, Line::from(spans))
}

fn segment_at(segments: &[Segment], sample: usize) -> Option<SegKind> {
    for s in segments {
        if sample >= s.start_sample && sample < s.end_sample {
            return Some(s.kind);
        }
    }
    None
}

fn blocks_for(level: f32) -> String {
    let chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let width = 28usize;
    let cells = ((level * width as f32).round() as usize).min(width);
    let mut s = String::with_capacity(width * 3);
    for i in 0..width {
        if i + 1 < cells {
            s.push('█');
        } else if i + 1 == cells {
            let idx = ((level * chars.len() as f32) as usize).min(chars.len() - 1);
            s.push(chars[idx]);
        } else {
            s.push(' ');
        }
    }
    s
}
