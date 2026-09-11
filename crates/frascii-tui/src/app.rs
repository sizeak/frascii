//! The event loop and the terminal's lifecycle.

use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::error::Result;

/// How long the loop waits for input before redrawing.
///
/// A ceiling on latency, not a frame budget: the loop redraws as soon as an
/// event arrives, so this only bounds how long an animated frame can be late
/// when nothing is happening. ~30fps.
const TICK: Duration = Duration::from_millis(33);

/// What the loop should do after handling an event.
///
/// An enum rather than a `bool` so that the pause, reset and fractal-switch
/// commands this will grow are additions rather than a second flag to keep
/// consistent with the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Keep running.
    Continue,
    /// Leave the loop and restore the terminal.
    Quit,
}

/// The frontend's state.
///
/// Deliberately thin at this stage. The camera, the fractal selection and the
/// animation clock are not here yet, and when they arrive the question to ask
/// of each is whether a second frontend would need the same decision — if so it
/// belongs in `frascii-core`, and only its *keybinding* belongs here.
#[derive(Debug, Default)]
pub struct App {}

impl App {
    /// A fresh app.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide what a key press means.
    ///
    /// Pure, and separate from [`App::run`] on purpose: this is the half worth
    /// testing, and a test that had to spawn a terminal to reach it would not
    /// get written. Every binding this grows should gain a case here.
    #[must_use]
    pub fn handle_key(&mut self, key: KeyEvent) -> Flow {
        // Only presses, because a key-up must not re-trigger the action its
        // key-down already ran: acting on `KeyEventKind::Release` would quit on
        // the *release* of the key that had already quit, and treating Repeat
        // as a press would make a held key fire twice.
        //
        // Worth being precise, because this check is *inert today* and that is
        // not obvious: on a Unix terminal the non-Press kinds arrive only if
        // the application pushes `REPORT_EVENT_TYPES` (`crossterm-0.29.0
        // src/event.rs:296-298`), and `ratatui::try_init` pushes no
        // keyboard-enhancement flags (`ratatui-0.30.2 src/init.rs:397-403`).
        // The other source is a Windows console, which emits Release on key-up
        // unprompted (`crossterm-0.29.0
        // src/event/sys/windows/parse.rs:226,289`) — and Windows is out of
        // scope for frascii.
        //
        // It stays because it stops being inert the moment we push
        // `REPORT_EVENT_TYPES`, which is how a pan/zoom UI gets unambiguous
        // modifier chords, and because a key-up that re-triggers a quit is a
        // bug worth three lines of prevention. `release_and_repeat_events_are_ignored`
        // is what documents the intent.
        if key.kind != KeyEventKind::Press {
            return Flow::Continue;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('c' | 'C'), KeyModifiers::CONTROL) => Flow::Quit,
            (KeyCode::Char('q' | 'Q') | KeyCode::Esc, _) => Flow::Quit,
            _ => Flow::Continue,
        }
    }

    /// Draw one frame.
    pub fn draw(&self, frame: &mut Frame<'_>) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " frascii ",
                Style::default().add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center);

        // `inner`, not `frame.area()`. Centring in the outer rect and clamping
        // to it lets a line as wide as the terminal paint straight over the left
        // and right border columns — which is exactly what the narrow-terminal
        // snapshots caught. Everything inside a bordered block measures against
        // its interior.
        let area = frame.area();
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(splash(), centred(inner, 44, 3));
    }

    /// Own the terminal until the user quits.
    ///
    /// Restoration is the caller's, via [`crate::run`]: a `run` that both
    /// initialised and restored the terminal could not be driven by a test
    /// harness's backend, and one that restored on the happy path only would
    /// leave a raw-mode terminal behind on the first error.
    pub fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;

            // One let-chain, and the short-circuit is load-bearing: `read`
            // blocks, so it must only be reached once `poll` has said an event
            // is waiting.
            if event::poll(TICK)?
                && let Event::Key(key) = event::read()?
                && self.handle_key(key) == Flow::Quit
            {
                tracing::debug!("quit requested");
                return Ok(());
            }
        }
    }
}

/// The placeholder frame: what frascii is, and how to leave.
///
/// This is the bootstrap's stand-in for the renderer. When the real render path
/// lands it replaces this entirely — the splash is not a start screen to be
/// kept around.
///
/// The version shown is *this crate's* (`CARGO_PKG_VERSION` resolves per
/// compiling package), which is correct only while the workspace shares one
/// version. Any user-facing version text that outlives this placeholder should
/// be passed in from the binary instead, since that is the version a user means
/// by "what have I got installed".
fn splash() -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from(Span::styled(
            concat!("frascii ", env!("CARGO_PKG_VERSION")),
            Style::default()
                .fg(Color::Rgb(0x7a, 0xd7, 0xff))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "realtime ascii fractals",
            Style::default().fg(Color::Rgb(0x9a, 0x9a, 0x9a)),
        )),
        Line::from(Span::styled(
            "press q to quit",
            Style::default().fg(Color::Rgb(0x6a, 0x6a, 0x6a)),
        )),
    ])
    .alignment(Alignment::Center)
}

/// A `width` x `height` rectangle centred in `area`, clamped to fit.
///
/// Clamped rather than asserted: a terminal narrower than the splash is a
/// normal thing to encounter, and the frame should shrink rather than the
/// arithmetic underflow.
fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn q_and_escape_quit() {
        let mut app = App::new();
        assert_eq!(app.handle_key(press(KeyCode::Char('q'))), Flow::Quit);
        assert_eq!(app.handle_key(press(KeyCode::Char('Q'))), Flow::Quit);
        assert_eq!(app.handle_key(press(KeyCode::Esc)), Flow::Quit);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = App::new();
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.handle_key(key), Flow::Quit);
    }

    #[test]
    fn a_bare_c_does_not_quit() {
        let mut app = App::new();
        assert_eq!(app.handle_key(press(KeyCode::Char('c'))), Flow::Continue);
    }

    #[test]
    fn unbound_keys_continue() {
        let mut app = App::new();
        assert_eq!(app.handle_key(press(KeyCode::Char('x'))), Flow::Continue);
        assert_eq!(app.handle_key(press(KeyCode::Up)), Flow::Continue);
    }

    #[test]
    fn release_and_repeat_events_are_ignored() {
        let mut app = App::new();
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let mut key = press(KeyCode::Char('q'));
            key.kind = kind;
            assert_eq!(app.handle_key(key), Flow::Continue, "{kind:?}");
        }
    }

    #[test]
    fn centred_rect_is_clamped_to_a_small_area() {
        let area = Rect::new(0, 0, 10, 2);
        assert_eq!(centred(area, 44, 3), Rect::new(0, 0, 10, 2));
    }

    #[test]
    fn centred_rect_respects_the_areas_origin() {
        let area = Rect::new(4, 6, 10, 6);
        assert_eq!(centred(area, 4, 2), Rect::new(7, 8, 4, 2));
    }
}
