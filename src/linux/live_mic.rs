//! Tells a microphone that delivers real audio from one that only opened.
//! A Bluetooth headset opens its stream in tens of milliseconds, then sends
//! exact zeros and codec noise for seconds before the microphone is live.

/// A 100 ms window is live when its RMS is above this (-91 dBFS). Measured
/// on a headset: codec noise stays at or below -94.8 dBFS per window, live
/// audio starts at -88.0 dBFS.
const LIVE_RMS: f32 = 2.818e-5;
/// Window length as a share of a second.
const WINDOWS_PER_SECOND: u32 = 10;

/// Watches a stream's chunks until one full window is above [`LIVE_RMS`].
#[derive(Debug, Clone)]
pub(crate) struct LiveMicGate {
    window: usize,
    filled: usize,
    sum_of_squares: f64,
    live: bool,
}

impl LiveMicGate {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self {
            window: (sample_rate / WINDOWS_PER_SECOND).max(1) as usize,
            filled: 0,
            sum_of_squares: 0.0,
            live: false,
        }
    }

    /// Adds a chunk of any size; true once the stream is live, and from then on.
    pub(crate) fn feed(&mut self, samples: &[f32]) -> bool {
        for &sample in samples {
            if self.live {
                break;
            }
            self.sum_of_squares += f64::from(sample) * f64::from(sample);
            self.filled += 1;
            if self.filled == self.window {
                let rms = (self.sum_of_squares / self.window as f64).sqrt();
                self.live = rms > f64::from(LIVE_RMS);
                self.filled = 0;
                self.sum_of_squares = 0.0;
            }
        }
        self.live
    }
}

/// A press that opened the microphone and waits for real audio before the
/// start sound.
#[derive(Debug, Default)]
pub(crate) struct LiveWait {
    /// Armed at the press, until the stream's first chunk.
    armed: bool,
    gate: Option<LiveMicGate>,
}

impl LiveWait {
    /// A press: waits for a live microphone when `on`.
    pub(crate) fn arm(&mut self, on: bool) {
        self.armed = on;
        self.gate = None;
    }

    /// Whether the start sound is held back and the HUD dimmed.
    pub(crate) fn waiting(&self) -> bool {
        self.armed || self.gate.is_some()
    }

    /// The stream's first chunk starts the capture: true when its start
    /// sound waits for [`Self::feed`].
    pub(crate) fn begin(&mut self, sample_rate: u32) -> bool {
        let waits = std::mem::take(&mut self.armed);
        self.gate = waits.then(|| LiveMicGate::new(sample_rate));
        waits
    }

    /// A recorded chunk; true once, when the microphone turns live.
    pub(crate) fn feed(&mut self, samples: &[f32]) -> bool {
        let live = self.gate.as_mut().is_some_and(|gate| gate.feed(samples));
        if live {
            self.gate = None;
        }
        live
    }

    /// The dictation ended or was dropped; true if it was still waiting.
    pub(crate) fn clear(&mut self) -> bool {
        let waited = self.waiting();
        self.armed = false;
        self.gate = None;
        waited
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn amplitude(dbfs: f32) -> f32 {
        10.0_f32.powf(dbfs / 20.0)
    }

    /// A square wave has an RMS equal to its amplitude.
    fn square(dbfs: f32, samples: usize) -> Vec<f32> {
        let level = amplitude(dbfs);
        (0..samples)
            .map(|n| if n % 2 == 0 { level } else { -level })
            .collect()
    }

    #[test]
    fn zeros_are_never_live() {
        let mut gate = LiveMicGate::new(RATE);
        for _ in 0..100 {
            assert!(!gate.feed(&[0.0; 4_800]));
        }
    }

    #[test]
    fn codec_noise_is_not_live() {
        let mut gate = LiveMicGate::new(RATE);
        for _ in 0..50 {
            assert!(!gate.feed(&square(-99.0, 4_800)));
        }
    }

    #[test]
    fn a_live_microphone_is_live_after_one_window() {
        let mut gate = LiveMicGate::new(RATE);
        assert!(!gate.feed(&square(-88.0, 4_799)));
        assert!(gate.feed(&square(-88.0, 1)));
        assert!(gate.feed(&[0.0; 10]), "stays live");
    }

    #[test]
    fn detects_across_many_small_chunks() {
        let mut gate = LiveMicGate::new(RATE);
        let noise = square(-99.0, 9_600);
        assert!(!gate.feed(&noise));
        let live = square(-88.0, 9_600);
        let mut detected_at = None;
        for (n, chunk) in live.chunks(37).enumerate() {
            if gate.feed(chunk) {
                detected_at = Some(n);
                break;
            }
        }
        assert!(detected_at.is_some());
    }

    #[test]
    fn a_window_mixing_noise_and_speech_counts_by_its_whole_rms() {
        let mut gate = LiveMicGate::new(RATE);
        // Windows are aligned from the start: half a window of zeros then
        // half of -80 dBFS averages to -83 dBFS, which is live.
        gate.feed(&[0.0; 2_400]);
        assert!(gate.feed(&square(-80.0, 2_400)));
    }

    #[test]
    fn a_press_waits_for_the_first_live_window_then_plays_once() {
        let mut wait = LiveWait::default();
        wait.arm(true);
        assert!(wait.waiting());
        assert!(wait.begin(RATE), "the start sound waits");
        assert!(!wait.feed(&[0.0; 4_800]));
        assert!(wait.waiting());
        assert!(wait.feed(&square(-88.0, 4_800)));
        assert!(!wait.waiting());
        assert!(!wait.feed(&square(-88.0, 4_800)), "reports live once");
    }

    #[test]
    fn without_the_wait_the_start_sound_is_not_held_back() {
        let mut wait = LiveWait::default();
        wait.arm(false);
        assert!(!wait.waiting());
        assert!(!wait.begin(RATE));
        assert!(!wait.feed(&square(-60.0, 4_800)));
    }

    #[test]
    fn clearing_ends_the_wait_before_or_after_the_first_chunk() {
        let mut wait = LiveWait::default();
        wait.arm(true);
        assert!(wait.clear(), "released before the first chunk");
        assert!(!wait.begin(RATE));
        wait.arm(true);
        wait.begin(RATE);
        assert!(wait.clear(), "released while silent");
        assert!(!wait.clear());
        assert!(!wait.feed(&square(-60.0, 4_800)));
    }
}
