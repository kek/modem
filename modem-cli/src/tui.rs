//! ratatui-based live reporter for `modem recv` / `modem send`.
//!
//! Activated when stdout is a TTY (and `--plain` isn't passed). Renders a
//! tone-bar meter, frame timeline, and status line. On drop, restores the
//! terminal to normal mode.

use std::io::{stdout, Stdout};
use std::time::Instant;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

use modem_codec::rx::FrameEvent;
use modem_core::fsk::{goertzel_bank, FskConfig};

use crate::reporter::{Reporter, RxEvent, TxEvent};
use crate::Profile;

const TONE_EMA_ALPHA: f32 = 0.4;
const DB_FLOOR: f32 = -80.0;
const DB_CEIL: f32 = 0.0;
const MAX_TIMELINE: usize = 64;

#[derive(Clone, Copy)]
enum FrameStatus { Ok, Dropped }

struct FrameMark {
    seq: u32,
    status: FrameStatus,
}

enum Mode {
    Rx {
        tones_db: Vec<f32>,
        timeline: Vec<FrameMark>,
        total_ok: usize,
        total_dropped: usize,
        rms: f32,
        listening_start: Instant,
    },
    Tx {
        bytes: usize,
        total_sec: f32,
        elapsed_sec: f32,
        tones_db: Vec<f32>,
    },
}

pub struct TuiReporter {
    term: Terminal<CrosstermBackend<Stdout>>,
    mode: Mode,
    freqs: Vec<f32>,
    sample_rate: u32,
}

fn cfg_for(profile: Profile) -> FskConfig {
    match profile {
        Profile::Audible => FskConfig::audible(),
        Profile::Ultrasonic => FskConfig::ultrasonic(),
    }
}

impl TuiReporter {
    pub fn new_rx(profile: Profile) -> std::io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let term = Terminal::new(backend)?;
        let cfg = cfg_for(profile);
        Ok(Self {
            term,
            mode: Mode::Rx {
                tones_db: vec![DB_FLOOR; cfg.tone_freqs.len()],
                timeline: Vec::new(),
                total_ok: 0,
                total_dropped: 0,
                rms: 0.0,
                listening_start: Instant::now(),
            },
            freqs: cfg.tone_freqs.to_vec(),
            sample_rate: cfg.sample_rate,
        })
    }

    pub fn new_tx(profile: Profile) -> std::io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let term = Terminal::new(backend)?;
        let cfg = cfg_for(profile);
        Ok(Self {
            term,
            mode: Mode::Tx {
                bytes: 0,
                total_sec: 0.0,
                elapsed_sec: 0.0,
                tones_db: vec![DB_FLOOR; cfg.tone_freqs.len()],
            },
            freqs: cfg.tone_freqs.to_vec(),
            sample_rate: cfg.sample_rate,
        })
    }

    pub fn should_quit(&mut self) -> bool {
        if event::poll(std::time::Duration::from_millis(0)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                if k.kind == KeyEventKind::Press {
                    return matches!(k.code, KeyCode::Char('q') | KeyCode::Esc);
                }
            }
        }
        false
    }

    pub fn set_tx_progress(&mut self, secs: f32) {
        if let Mode::Tx { elapsed_sec, .. } = &mut self.mode {
            *elapsed_sec = secs;
        }
    }

    fn update_tones(&mut self, samples: &[f32]) {
        let mags = goertzel_bank(samples, &self.freqs, self.sample_rate);
        let norm = (samples.len().max(1) as f32).powi(2);
        match &mut self.mode {
            Mode::Rx { tones_db, rms, .. } => {
                let mut sum_sq = 0.0f64;
                for x in samples { sum_sq += (*x as f64) * (*x as f64); }
                *rms = (sum_sq / samples.len().max(1) as f64).sqrt() as f32;
                for (i, m) in mags.iter().enumerate() {
                    let v = m / norm;
                    let db = if v > 0.0 { 10.0 * v.log10() } else { DB_FLOOR };
                    tones_db[i] += TONE_EMA_ALPHA * (db - tones_db[i]);
                }
            }
            Mode::Tx { tones_db, .. } => {
                for (i, m) in mags.iter().enumerate() {
                    let v = m / norm;
                    let db = if v > 0.0 { 10.0 * v.log10() } else { DB_FLOOR };
                    tones_db[i] += TONE_EMA_ALPHA * (db - tones_db[i]);
                }
            }
        }
    }

    fn draw_rx(&mut self) {
        let (tones, timeline, total_ok, total_dropped, rms, elapsed) = match &self.mode {
            Mode::Rx { tones_db, timeline, total_ok, total_dropped, rms, listening_start } => (
                tones_db.clone(),
                timeline.iter().map(|m| (m.seq, m.status)).collect::<Vec<_>>(),
                *total_ok,
                *total_dropped,
                *rms,
                listening_start.elapsed().as_secs_f32(),
            ),
            _ => return,
        };
        let freqs = self.freqs.clone();
        let _ = self.term.draw(|f| {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(11),
                    Constraint::Length(3),
                    Constraint::Length(3),
                ])
                .split(f.area());

            let mut bar_lines = String::new();
            for (db, hz) in tones.iter().zip(freqs.iter()) {
                let level = ((*db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0);
                bar_lines.push_str(&format!("{:>4.1}k  {}\n", hz / 1000.0, blocks_for(level)));
            }
            let bars_p = Paragraph::new(bar_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" tones · rms {:.3} ", rms))
            );
            f.render_widget(bars_p, layout[0]);

            let chips: Vec<Span> = timeline.iter().flat_map(|(seq, status)| {
                let (label, color) = match status {
                    FrameStatus::Ok => (format!("[ok {seq}]"), Color::Green),
                    FrameStatus::Dropped => (format!("[DROP {seq}]"), Color::Red),
                };
                vec![Span::styled(label, Style::default().fg(color)), Span::raw(" ")]
            }).collect();
            let tl = Paragraph::new(Line::from(chips)).block(
                Block::default().borders(Borders::ALL).title(" frames ")
            );
            f.render_widget(tl, layout[1]);

            let status = format!(
                " listening {elapsed:.1} s · {total_ok} ok · {total_dropped} dropped · q to quit "
            );
            f.render_widget(Paragraph::new(status), layout[2]);
        });
    }

    fn draw_tx(&mut self) {
        let (bytes, total_sec, elapsed_sec, tones) = match &self.mode {
            Mode::Tx { bytes, total_sec, elapsed_sec, tones_db } => {
                (*bytes, *total_sec, *elapsed_sec, tones_db.clone())
            }
            _ => return,
        };
        let freqs = self.freqs.clone();
        let _ = self.term.draw(|f| {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Length(11)])
                .split(f.area());
            let frac = if total_sec > 0.0 { (elapsed_sec / total_sec).clamp(0.0, 1.0) } else { 0.0 };
            let pct = (frac * 100.0).round() as u32;
            let width = 40usize;
            let filled = ((width as f32) * frac) as usize;
            let bar = format!(
                "[{}{}] {pct}% · {bytes} B · {elapsed_sec:.1}/{total_sec:.1} s",
                "█".repeat(filled),
                "░".repeat(width.saturating_sub(filled)),
            );
            f.render_widget(
                Paragraph::new(bar).block(Block::default().borders(Borders::ALL).title(" sending ")),
                layout[0],
            );
            let mut bar_lines = String::new();
            for (db, hz) in tones.iter().zip(freqs.iter()) {
                let level = ((*db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0);
                bar_lines.push_str(&format!("{:>4.1}k  {}\n", hz / 1000.0, blocks_for(level)));
            }
            f.render_widget(
                Paragraph::new(bar_lines).block(Block::default().borders(Borders::ALL).title(" tones ")),
                layout[1],
            );
        });
    }
}

fn blocks_for(level: f32) -> String {
    let chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let width = 28usize;
    let cells = (level * width as f32).round() as usize;
    let cells = cells.min(width);
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

impl Drop for TuiReporter {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

impl crate::reporter::TxProgressSink for TuiReporter {
    fn set_tx_progress(&mut self, elapsed_sec: f32) {
        TuiReporter::set_tx_progress(self, elapsed_sec);
    }
}

impl Reporter for TuiReporter {
    fn as_tx_progress_sink(&mut self) -> Option<&mut dyn crate::reporter::TxProgressSink> {
        Some(self)
    }

    fn on_rx(&mut self, e: RxEvent<'_>) {
        match e {
            RxEvent::Listening => self.draw_rx(),
            RxEvent::Chunk(samples) => {
                self.update_tones(samples);
                self.draw_rx();
            }
            RxEvent::Frame(ev) => {
                if let Mode::Rx { timeline, total_ok, total_dropped, .. } = &mut self.mode {
                    match ev {
                        FrameEvent::FrameOk { seq, .. } => {
                            if timeline.len() >= MAX_TIMELINE { timeline.remove(0); }
                            timeline.push(FrameMark { seq: *seq, status: FrameStatus::Ok });
                            *total_ok += 1;
                        }
                        FrameEvent::FrameDropped { seq, .. } => {
                            if timeline.len() >= MAX_TIMELINE { timeline.remove(0); }
                            timeline.push(FrameMark { seq: *seq, status: FrameStatus::Dropped });
                            *total_dropped += 1;
                        }
                        FrameEvent::StreamComplete { .. } => {}
                    }
                }
                self.draw_rx();
            }
        }
    }

    fn on_tx(&mut self, e: TxEvent<'_>) {
        match e {
            TxEvent::Starting { bytes, samples: _, seconds } => {
                if let Mode::Tx { bytes: b, total_sec, .. } = &mut self.mode {
                    *b = bytes;
                    *total_sec = seconds;
                }
                self.draw_tx();
            }
            TxEvent::Chunk(samples) => {
                self.update_tones(samples);
                self.draw_tx();
            }
            TxEvent::Done => self.draw_tx(),
        }
    }
}
