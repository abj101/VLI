//! Lightweight snap detector for 16 kHz mono audio.
//!
//! Everything is measured on the high-passed (>~1.5 kHz) signal. Real snaps recorded on the
//! target mic leave a 300+ ms low-frequency rumble and room tail, so full-band "back to quiet"
//! checks rejected every one. The bright band is clean: a ~40 dB jump, ≥10 dB drop within
//! 10 ms of the peak, then ≥20 dB down within ~50 ms.

const HOP: usize = 80; // 5 ms
const QUIET_HOPS: usize = 30;
const ISOLATION_HOPS: usize = 30;
const COOLDOWN_HOPS: usize = 100;
/// Diagnostic events start this far over the floor; detection uses `sensitivity_db`.
const EVENT_DB: f32 = 10.0;
/// The bright "crack": drop this far below peak within `CRACK_HOPS` of it.
const CRACK_DB: f32 = 10.0;
const CRACK_HOPS: usize = 4;
/// Tail must fall this far below peak within `DECAY_HOPS`.
const DECAY_DB: f32 = 20.0;
const DECAY_HOPS: usize = 30;
const MIN_HIGH_RATIO: f32 = 0.60;
const FLOOR_MIN: f32 = 1.0e-3;
const HIGH_PASS_ALPHA: f32 = 0.555;

fn to_db(x: f32) -> f32 {
    20.0 * x.max(FLOOR_MIN).log10()
}

/// One loud transient, followed from its first hop to its resolution.
#[derive(Clone, Copy)]
struct Event {
    floor_db: f32,
    quiet_hops_before: usize,
    age: usize,
    peak_db: f32,
    peak_age: usize,
    high_ratio: f32,
    cracked: bool,
    decay_ms: Option<usize>,
    /// Known-bad at onset (cooldown / not quiet before); still followed so it logs once.
    rejected: Option<&'static str>,
}

impl Event {
    fn log(&self, outcome: &str) {
        let decay = self.decay_ms.map_or_else(|| "-".to_owned(), |ms| ms.to_string());
        log::debug!(
            "snap event: onset_db={:.1} high_ratio={:.3} floor={:.1} dBFS quiet_hops_before={} decay_ms={} outcome={}",
            self.peak_db - self.floor_db, self.high_ratio, self.floor_db, self.quiet_hops_before, decay, outcome
        );
    }
}

/// Detects isolated, bright, short transients in contiguous 16 kHz mono PCM.
pub(super) struct SnapDetector {
    sensitivity_db: f32,
    /// Noise floor of the high-passed signal (RMS).
    floor: f32,
    samples: usize,
    energy: f32,
    high_energy: f32,
    low_pass: f32,
    quiet_hops: usize,
    loud_hops: usize,
    last_quiet_run: usize,
    event: Option<Event>,
    pending: Option<(Event, usize)>,
    latched: bool,
    cooldown_hops: usize,
}

impl SnapDetector {
    pub(super) fn new(sensitivity_db: f32) -> Self {
        Self {
            sensitivity_db: sensitivity_db.clamp(10.0, 30.0),
            floor: FLOOR_MIN,
            samples: 0,
            energy: 0.0,
            high_energy: 0.0,
            low_pass: 0.0,
            quiet_hops: 0,
            loud_hops: 0,
            last_quiet_run: 0,
            event: None,
            pending: None,
            latched: false,
            cooldown_hops: 0,
        }
    }

    /// Feed any-sized contiguous audio chunk. Returns true once the isolated snap is confirmed.
    pub(super) fn process(&mut self, pcm: &[f32]) -> bool {
        let mut fired = false;
        for &sample in pcm {
            let sample = if sample.is_finite() { sample } else { 0.0 };
            let low = self.low_pass + (1.0 - HIGH_PASS_ALPHA) * (sample - self.low_pass);
            let high = sample - low;
            self.low_pass = low;
            self.energy += sample * sample;
            self.high_energy += high * high;
            self.samples += 1;
            if self.samples == HOP {
                fired |= self.process_hop();
                self.samples = 0;
                self.energy = 0.0;
                self.high_energy = 0.0;
            }
        }
        fired
    }

    /// Clear audio history after pause/disable so stale partial events cannot fire later.
    pub(super) fn reset(&mut self) {
        *self = Self::new(self.sensitivity_db);
    }

    fn process_hop(&mut self) -> bool {
        let high_rms = (self.high_energy / HOP as f32).sqrt();
        // Calibrate the one-pole high-pass energy against the >1.5 kHz band share.
        let high_ratio = (self.high_energy / self.energy.max(1.0e-20) * 2.0).clamp(0.0, 1.0);
        let db = to_db(high_rms);
        let floor_db = to_db(self.floor);
        let quiet = high_rms <= self.floor * 2.0;
        let mut fired = false;

        self.cooldown_hops = self.cooldown_hops.saturating_sub(1);

        // Isolation: nothing near the snap's own level while its tail dies away. The tail
        // itself bounces by 10 dB, so only comparing against the peak is reliable.
        if let Some((event, waited)) = self.pending.take() {
            if db > event.peak_db - CRACK_DB {
                event.log("not_isolated");
            } else if waited + 1 >= ISOLATION_HOPS {
                self.cooldown_hops = COOLDOWN_HOPS;
                event.log("detected");
                fired = true;
            } else {
                self.pending = Some((event, waited + 1));
            }
        }

        if let Some(mut e) = self.event.take() {
            e.age += 1;
            if db > e.peak_db {
                e.peak_db = db;
                e.peak_age = e.age;
                e.high_ratio = high_ratio;
                e.cracked = false;
            } else if e.age - e.peak_age <= CRACK_HOPS && db <= e.peak_db - CRACK_DB {
                e.cracked = true;
            }
            if quiet || db <= e.peak_db - DECAY_DB {
                e.decay_ms = Some(e.age * 5);
                let outcome = e.rejected.or(if e.peak_db - e.floor_db < self.sensitivity_db {
                    Some("below_sensitivity")
                } else if e.high_ratio < MIN_HIGH_RATIO {
                    Some("not_bright")
                } else if !e.cracked {
                    Some("no_crack")
                } else {
                    None
                });
                match outcome {
                    Some(outcome) => e.log(outcome),
                    None => self.pending = Some((e, 0)),
                }
            } else if e.age >= DECAY_HOPS {
                e.log(e.rejected.unwrap_or("sustained"));
            } else {
                self.event = Some(e);
            }
        } else if self.pending.is_none() && !self.latched && db - floor_db >= EVENT_DB {
            // A snap often starts mid-hop: one partly-loud hop before it still counts as quiet.
            let quiet_hops_before = match self.loud_hops {
                0 => self.quiet_hops,
                1 => self.last_quiet_run,
                _ => 0,
            };
            self.latched = true;
            self.event = Some(Event {
                floor_db,
                quiet_hops_before,
                age: 0,
                peak_db: db,
                peak_age: 0,
                high_ratio,
                cracked: false,
                decay_ms: None,
                rejected: if self.cooldown_hops > 0 {
                    Some("cooldown")
                } else if quiet_hops_before < QUIET_HOPS {
                    Some("not_quiet_before")
                } else {
                    None
                },
            });
        }

        if quiet {
            self.quiet_hops += 1;
            self.loud_hops = 0;
        } else {
            if self.quiet_hops > 0 {
                self.last_quiet_run = self.quiet_hops;
            }
            self.quiet_hops = 0;
            self.loud_hops += 1;
        }
        if self.event.is_none() && self.pending.is_none() {
            if quiet {
                self.latched = false;
            }
            // Adapt outside transients so snaps never raise their own threshold.
            self.floor = (self.floor * 0.99 + high_rms * 0.01).max(FLOOR_MIN);
        }
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::{SnapDetector, FLOOR_MIN};

    // Keep captured records local to this test thread, even when audio tests run in parallel.
    std::thread_local! {
        static EVENT_LINES: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
    }

    struct EventLogger;

    impl log::Log for EventLogger {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            EVENT_LINES.with(|lines| {
                if let Some(lines) = lines.borrow_mut().as_mut() {
                    lines.push(record.args().to_string());
                }
            });
        }

        fn flush(&self) {}
    }

    #[test]
    fn logs_one_complete_record_per_transient_for_every_outcome() {
        static LOGGER: EventLogger = EventLogger;
        log::set_logger(&LOGGER).expect("install snap test logger");
        log::set_max_level(log::LevelFilter::Debug);

        // Bright alternating PCM gives a reproducible short transient; the sine is a thump.
        let quiet = vec![0.0; super::HOP];
        let bright: Vec<_> = (0..super::HOP)
            .map(|i| if i % 2 == 0 { 0.1 } else { -0.1 })
            .collect();
        let soft: Vec<_> = bright.iter().map(|x| x * 0.05).collect();
        let inaudible: Vec<_> = bright.iter().map(|x| x * 0.02).collect();
        let dull: Vec<_> = (0..super::HOP * 2)
            .map(|i| 0.8 * (2.0 * std::f32::consts::PI * 100.0 * i as f32 / 16_000.0).sin())
            .collect();

        let cases: Vec<(Vec<(&[f32], usize)>, Vec<&str>, usize)> = vec![
            (
                vec![(&quiet, 40), (&bright, 2), (&quiet, 40)],
                vec!["detected"],
                1,
            ),
            (
                vec![(&quiet, 40), (&soft, 2), (&quiet, 40)],
                vec!["below_sensitivity"],
                0,
            ),
            (
                vec![(&quiet, 40), (&dull, 1), (&quiet, 40)],
                vec!["not_bright"],
                0,
            ),
            (
                vec![(&bright, 2), (&quiet, 40)],
                vec!["not_quiet_before"],
                0,
            ),
            (
                vec![(&quiet, 40), (&bright, 14), (&quiet, 40)],
                vec!["no_crack"],
                0,
            ),
            (
                vec![(&quiet, 40), (&bright, 40), (&quiet, 40)],
                vec!["sustained"],
                0,
            ),
            (
                vec![
                    (&quiet, 40),
                    (&bright, 2),
                    (&quiet, 5),
                    (&bright, 2),
                    (&quiet, 40),
                ],
                vec!["not_isolated"],
                0,
            ),
            (
                vec![
                    (&quiet, 40),
                    (&bright, 2),
                    (&quiet, 40),
                    (&bright, 2),
                    (&quiet, 40),
                ],
                vec!["detected", "cooldown"],
                1,
            ),
            (vec![(&quiet, 40), (&inaudible, 2), (&quiet, 40)], vec![], 0),
        ];

        for (parts, outcomes, expected_fires) in cases {
            let signal: Vec<f32> = parts
                .into_iter()
                .flat_map(|(pcm, count)| pcm.repeat(count))
                .collect();
            EVENT_LINES.with(|lines| *lines.borrow_mut() = Some(Vec::new()));
            assert_eq!(fires_in_chunks(&signal, 113), expected_fires);
            let lines = EVENT_LINES.with(|lines| lines.borrow_mut().take().unwrap());
            assert_eq!(lines.len(), outcomes.len(), "{lines:?}");
            for (line, outcome) in lines.iter().zip(outcomes) {
                assert!(line.starts_with("snap event: onset_db="), "{line}");
                for field in [
                    " high_ratio=",
                    " floor=",
                    " dBFS quiet_hops_before=",
                    " decay_ms=",
                ] {
                    assert!(line.contains(field), "{line}");
                }
                assert!(line.ends_with(&format!(" outcome={outcome}")), "{line}");
                if outcome == "detected" {
                    assert!(line.contains("high_ratio=1.000 floor=-60.0 dBFS quiet_hops_before=40 decay_ms=10"), "{line}");
                } else if outcome == "sustained" {
                    assert!(line.contains(" decay_ms=- "), "{line}");
                } else if outcome == "no_crack" {
                    assert!(line.contains(" decay_ms=70 "), "{line}");
                }
            }
        }
    }

    fn noise_snap(signal: &mut [f32], start: usize, amplitude: f32) {
        let mut seed = 0x1234_5678u32;
        for (offset, sample) in signal[start..].iter_mut().take(1600).enumerate() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed as i32 as f32) / i32::MAX as f32;
            let envelope = if offset < 32 {
                1.0
            } else {
                (-((offset - 32) as f32) / 160.0).exp()
            };
            *sample += amplitude * noise * envelope;
        }
    }

    fn noise_background(signal: &mut [f32], amplitude: f32) {
        let mut seed = 0x9abc_def0u32;
        for sample in signal {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *sample = amplitude * (seed as i32 as f32 / i32::MAX as f32);
        }
    }

    fn fires_in_chunks(signal: &[f32], chunk: usize) -> usize {
        let mut detector = SnapDetector::new(18.0);
        signal
            .chunks(chunk)
            .filter(|part| detector.process(part))
            .count()
    }

    #[test]
    fn isolated_bright_snap_fires_once_across_chunk_boundaries() {
        let mut signal = vec![0.0; 16_000];
        noise_snap(&mut signal, 4_000, 0.5);
        assert_eq!(fires_in_chunks(&signal, 137), 1);
        assert_eq!(fires_in_chunks(&signal, 16_000), 1);
    }

    #[test]
    fn adapts_to_startup_noise_then_detects_snap_over_it() {
        let mut signal = vec![0.0; 32_000];
        noise_background(&mut signal, 0.04);
        noise_snap(&mut signal, 12_800, 0.5);
        assert_eq!(fires_in_chunks(&signal, 320), 1);
    }

    #[test]
    fn noise_floor_tracks_up_and_back_down() {
        let mut detector = SnapDetector::new(18.0);
        let mut noise = vec![0.0; 16_000];
        noise_background(&mut noise, 0.04);
        detector.process(&noise);
        assert!(detector.floor > 0.01);

        detector.process(&vec![0.0; 48_000]);
        assert!(detector.floor <= FLOOR_MIN * 1.1);
    }

    #[test]
    fn detects_snaps_at_each_offset_within_a_hop() {
        for offset in 1..80 {
            let mut signal = vec![0.0; 12_000];
            noise_snap(&mut signal, 4_000 + offset, 0.5);
            assert_eq!(fires_in_chunks(&signal, 113), 1, "offset {offset}");
        }
    }

    #[test]
    fn low_thump_and_sustained_tone_do_not_fire() {
        let mut thump = vec![0.0; 16_000];
        for (offset, sample) in thump[4_000..].iter_mut().take(1600).enumerate() {
            let t = offset as f32 / 16_000.0;
            *sample = 0.8 * (2.0 * std::f32::consts::PI * 100.0 * t).sin() * (-t / 0.01).exp();
        }
        assert_eq!(fires_in_chunks(&thump, 257), 0);

        let mut tone = vec![0.0; 16_000];
        for (i, sample) in tone[4_000..].iter_mut().take(8_000).enumerate() {
            *sample = 0.2 * (2.0 * std::f32::consts::PI * 200.0 * i as f32 / 16_000.0).sin();
        }
        assert_eq!(fires_in_chunks(&tone, 80), 0);
    }

    #[test]
    fn cooldown_suppresses_second_snap_300_ms_later() {
        let mut signal = vec![0.0; 16_000];
        noise_snap(&mut signal, 3_000, 0.5);
        noise_snap(&mut signal, 7_800, 0.5);
        assert_eq!(fires_in_chunks(&signal, 320), 1);
    }

    #[test]
    fn closely_spaced_clicks_fail_isolation_and_prequiet_checks() {
        let mut signal = vec![0.0; 16_000];
        for start in (3_000..).step_by(1_280).take(5) {
            noise_snap(&mut signal, start, 0.5);
        }
        assert_eq!(fires_in_chunks(&signal, 191), 0);
    }

    #[test]
    fn slow_broadband_clap_decay_does_not_fire() {
        let mut signal = vec![0.0; 16_000];
        let mut seed = 0x8765_4321u32;
        for (offset, sample) in signal[4_000..].iter_mut().take(4_000).enumerate() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = seed as i32 as f32 / i32::MAX as f32;
            *sample = 0.5 * noise * (-(offset as f32) / 1_280.0).exp();
        }
        assert_eq!(fires_in_chunks(&signal, 173), 0);
    }

    /// Shape of real recorded snaps: a bright crack plus a long 40 Hz rumble and a slow
    /// room tail. Full-band decay checks rejected all 21 snaps in the capture.
    #[test]
    fn snap_with_rumble_and_room_tail_fires() {
        let mut signal = vec![0.0; 16_000];
        noise_snap(&mut signal, 4_000, 0.5);
        for (i, sample) in signal[4_000..].iter_mut().take(6_400).enumerate() {
            let t = i as f32 / 16_000.0;
            *sample += 0.05 * (2.0 * std::f32::consts::PI * 40.0 * t).sin();
        }
        assert_eq!(fires_in_chunks(&signal, 160), 1);
    }

    #[test]
    fn reset_discards_partial_hop_and_candidate_history() {
        let mut detector = SnapDetector::new(18.0);
        let mut signal = vec![0.0; 5_000];
        noise_snap(&mut signal, 3_000, 0.5);
        detector.process(&signal[..3_200]);
        detector.reset();
        assert!(!detector.process(&signal[3_200..]));
    }
}
