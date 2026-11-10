//! The microphone stream's clock on the Linux listener.

use std::time::{Duration, Instant};

use crate::audio::CaptureInstant;

/// Where the microphone stream's clock stands between chunks: the newest
/// chunk's end plus the time since it arrived. PipeWire and ALSA can deliver
/// large chunks (#1309), so the newest audio may trail a press by most of
/// one; shortcut presses and releases are placed here instead.
#[derive(Default)]
pub(super) struct StreamClock {
    newest: Option<(CaptureInstant, Instant)>,
}

impl StreamClock {
    /// A stalled stream's lag is not counted past this.
    pub(super) const MAX_LAG: Duration = Duration::from_secs(2);

    pub(super) fn chunk(&mut self, captured_through: CaptureInstant, received: Instant) {
        self.newest = Some((captured_through, received));
    }

    /// A replaced or reopened stream keeps its own clock: none of the old
    /// one is kept.
    pub(super) fn reset(&mut self) {
        self.newest = None;
    }

    /// The end of the newest chunk.
    pub(super) fn through(&self) -> CaptureInstant {
        self.newest
            .map_or(CaptureInstant::ZERO, |(through, _)| through)
    }

    /// The stream time at `now`; `None` before this stream's first chunk.
    pub(super) fn at(&self, now: Instant) -> Option<CaptureInstant> {
        let (through, received) = self.newest?;
        Some(through + now.saturating_duration_since(received).min(Self::MAX_LAG))
    }
}
