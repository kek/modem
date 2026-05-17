//! Interactive chat: type a line, press Enter to transmit. Mic input is
//! decoded concurrently so received messages appear in the log. Shares the
//! tone-bar + frame-timeline visualization with `recv`/`send`.

use crate::{make_phy, Profile};
use modem_audio::input::open_mic;
use modem_codec::rx::{FrameEvent, Receiver};
use modem_codec::tx::Transmitter;
use modem_core::fsk::{goertzel_bank, DspVariants, FskConfig};
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
const CHUNK_SAMPLES: usize = 2400; // 50 ms at 48 kHz

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
        while let Ok(chunk) = mic.rx.try_recv() {
            if tx_state.is_none() {
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
            let elapsed = t.start.elapsed().as_secs_f32();
            let want = ((elapsed / t.total_sec.max(1e-6)) * t.samples.len() as f32) as usize;
            let want = want.min(t.samples.len());
            if want > t.cursor {
                update_tones(&mut tones_db, &mut rms, &t.samples[t.cursor..want], &freqs, sample_rate);
                t.cursor = want;
            }
            if t.done_rx.try_recv().is_ok() {
                tx_state = None;
                // Reset the RX decoder so any residual mic buffer (echo of
                // our outgoing) doesn't get misinterpreted as a fresh frame.
                rx = Receiver::new(make_phy(profile, variants));
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
                            if !input.is_empty() && tx_state.is_none() {
                                let bytes = input.as_bytes().to_vec();
                                let samples = tx.encode(&bytes);
                                let total_sec = samples.len() as f32 / sample_rate as f32;
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
            let elapsed_tx = tx_state.as_ref().map(|t| t.start.elapsed().as_secs_f32());
            let (tx_bytes, tx_total, tx_elapsed) = match tx_state.as_ref() {
                Some(t) => (Some(t.bytes), Some(t.total_sec), elapsed_tx),
                None => (None, None, None),
            };
            term.draw(|f| {
                let area = f.area();
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(11),
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
                    (Some(b), Some(t), Some(e)) => format!(
                        " tones · rms {:.3} · sending {} B {:.1}/{:.1} s ",
                        rms, b, e, t,
                    ),
                    _ => format!(" tones · rms {:.3} ", rms),
                };
                f.render_widget(
                    Paragraph::new(bar_lines).block(Block::default().borders(Borders::ALL).title(title)),
                    layout[0],
                );

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
                    layout[1],
                );

                // Message log (most recent at bottom, scrolling).
                let log_height = layout[2].height.saturating_sub(2) as usize;
                let log_start = log.len().saturating_sub(log_height);
                let in_flight = tx_state.as_ref().map(|t| {
                    let progress = (t.cursor as f32 / t.samples.len().max(1) as f32).clamp(0.0, 1.0);
                    (t.log_pos, progress)
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
                    layout[2],
                );

                // Input prompt.
                let prompt = format!("> {input}");
                f.render_widget(
                    Paragraph::new(prompt.clone()).block(Block::default().borders(Borders::ALL).title(" type ")),
                    layout[3],
                );
                // Place the terminal cursor after the prompt + input text.
                let cursor_x = layout[3].x + 1 + (prompt.chars().count() as u16);
                let cursor_y = layout[3].y + 1;
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
