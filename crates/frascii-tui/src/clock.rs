//! Animation time.

use std::time::{Duration, Instant};

/// A source of per-frame time deltas that respects pausing.
///
/// It reports **gaps between frames**, never time since a start instant. Every
/// rate in the renderer is expressed per second and applied per frame, so a
/// delta is all any of them need — and deriving one by subtracting from a start
/// would make a paused minute arrive as a minute of animation the moment the
/// user resumed.
///
/// There is deliberately no accumulated total: nothing reads one, and a
/// counter nobody reads is the kind of thing this codebase has already deleted
/// once. (The bootstrap's version was worse than unused — it incremented once
/// per *idle poll*, so any keypress skipped a tick and it ran slower the more
/// you interacted with it.)
#[derive(Debug, Clone, Copy)]
pub(crate) struct Clock {
    /// When `tick` was last called, whether paused or not.
    last: Option<Instant>,
    paused: bool,
}

impl Clock {
    /// A running clock at zero.
    pub(crate) const fn new() -> Self {
        Self {
            last: None,
            paused: false,
        }
    }

    /// Advance to `now`, returning how much animation time passed.
    ///
    /// Zero while paused — but `last` still moves, which is the point: on
    /// unpausing, the first `dt` is one frame rather than however long the
    /// pause lasted. Getting that wrong makes an animation lurch every time it
    /// resumes.
    ///
    /// The first call returns zero, since there is no previous instant to
    /// measure from, and a made-up first `dt` would make the opening frame of
    /// any motion inconsistent.
    pub(crate) fn tick(&mut self, now: Instant) -> Duration {
        let dt = match self.last {
            Some(last) if !self.paused => now.saturating_duration_since(last),
            _ => Duration::ZERO,
        };
        self.last = Some(now);
        dt
    }

    /// Whether the clock is paused.
    pub(crate) const fn is_paused(self) -> bool {
        self.paused
    }

    /// Pause or resume.
    pub(crate) const fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_tick_yields_no_time() {
        // There is nothing to measure from, and inventing a first `dt` would
        // make the opening frame of an animation inconsistent with the rest.
        let mut clock = Clock::new();
        assert_eq!(clock.tick(Instant::now()), Duration::ZERO);
    }

    #[test]
    fn ticking_accumulates_the_gaps() {
        let start = Instant::now();
        let mut clock = Clock::new();
        clock.tick(start);
        assert_eq!(
            clock.tick(start + Duration::from_millis(30)),
            Duration::from_millis(30)
        );
        assert_eq!(
            clock.tick(start + Duration::from_millis(50)),
            Duration::from_millis(20)
        );
    }

    #[test]
    fn a_pause_contributes_no_time_at_all() {
        let start = Instant::now();
        let mut clock = Clock::new();
        clock.tick(start);
        clock.tick(start + Duration::from_millis(10));

        clock.toggle_pause();
        assert!(clock.is_paused());
        assert_eq!(clock.tick(start + Duration::from_secs(60)), Duration::ZERO);
        // And a second paused frame is still zero, so the pause does not leak
        // in one frame at a time.
        assert_eq!(clock.tick(start + Duration::from_secs(90)), Duration::ZERO);
    }

    #[test]
    fn resuming_does_not_lurch_forward_by_the_pause() {
        // The reason this accumulates rather than subtracting from a start
        // instant: a paused minute must not become a minute of animation the
        // moment the user unpauses.
        let start = Instant::now();
        let mut clock = Clock::new();
        clock.tick(start);
        clock.toggle_pause();
        clock.tick(start + Duration::from_secs(60));
        clock.toggle_pause();

        let dt = clock.tick(start + Duration::from_millis(60_030));
        assert_eq!(
            dt,
            Duration::from_millis(30),
            "resumed with a lurch: {dt:?}"
        );
    }

    #[test]
    fn a_clock_that_goes_backwards_yields_zero_rather_than_panicking() {
        // `Instant` is monotonic, so this should not happen — but
        // `saturating_duration_since` costs nothing and a subtraction overflow
        // panic in the render loop would be a poor trade.
        let start = Instant::now();
        let mut clock = Clock::new();
        clock.tick(start + Duration::from_secs(10));
        assert_eq!(clock.tick(start), Duration::ZERO);
    }
}
