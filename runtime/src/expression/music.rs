//! Small PCM onset tracker. The mixer advances it on the audio sample clock.
use serde::Serialize;
use std::collections::VecDeque;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct MusicPulse {
    pub bpm: f64,
    pub beats: f64,
    pub energy: f64,
    pub onset_strength: f64,
    pub audible: bool,
    pub streaming: bool,
    pub locked: bool,
    #[serde(skip)]
    pub sampled_at: Option<Instant>,
}

pub struct BeatTracker {
    low: [f64; 2],
    mid: [f64; 2],
    previous: [f64; 3],
    flux: VecDeque<f64>,
    onsets: VecDeque<f64>,
    seconds: f64,
    quiet: f64,
    period: f64,
    beats: f64,
    energy: f64,
    frames: u64,
}
impl Default for BeatTracker {
    fn default() -> Self {
        Self {
            low: [0.; 2],
            mid: [0.; 2],
            previous: [0.; 3],
            flux: VecDeque::new(),
            onsets: VecDeque::new(),
            seconds: 0.,
            quiet: 0.,
            period: 0.5,
            beats: 0.,
            energy: 0.,
            frames: 0,
        }
    }
}
impl BeatTracker {
    pub fn process(&mut self, pcm: &[[f32; 2]]) -> MusicPulse {
        let dt = pcm.len() as f64 / 48_000.;
        let mut bands = [0.; 3];
        let mut power = 0.;
        // Three inexpensive filter bands retain low drum and high transient
        // onsets without an FFT, allocations per sample, or a network request.
        const LOW: f64 = 0.019443443;
        const MID: f64 = 0.230334588;
        for frame in pcm {
            for channel in 0..2 {
                let sample = f64::from(frame[channel]);
                self.low[channel] += LOW * (sample - self.low[channel]);
                self.mid[channel] += MID * (sample - self.mid[channel]);
                for (index, value) in [
                    self.low[channel],
                    self.mid[channel] - self.low[channel],
                    sample - self.mid[channel],
                ]
                .into_iter()
                .enumerate()
                {
                    bands[index] += value * value;
                }
                power += sample * sample;
            }
        }
        let count = (pcm.len() * 2).max(1) as f64;
        let rms = (power / count).sqrt();
        let mut flux = 0.;
        for (index, band) in bands.into_iter().enumerate() {
            let level = (band / count).sqrt();
            flux += (level - self.previous[index]).max(0.);
            self.previous[index] = level;
        }
        self.seconds += dt;
        self.frames += 1;
        let mean =
            self.flux.iter().rev().take(100).sum::<f64>() / self.flux.len().min(100).max(1) as f64;
        let onset = flux > (mean * 2.5).max(0.002)
            && rms > 0.001
            && self
                .onsets
                .back()
                .is_none_or(|last| self.seconds - last > 0.22);
        if onset {
            if self.onsets.is_empty() {
                self.beats = 0.;
            } else {
                let error = self.beats.round() - self.beats;
                if error.abs() < 0.2 {
                    self.beats += error * 0.2;
                }
            }
            self.onsets.push_back(self.seconds);
        }
        while self
            .onsets
            .front()
            .is_some_and(|time| self.seconds - time > 12.)
        {
            self.onsets.pop_front();
        }
        self.flux.push_back(flux);
        if self.flux.len() > 1200 {
            self.flux.pop_front();
        }
        if self.frames % 25 == 0 && self.onsets.len() >= 4 {
            self.track_tempo();
        }
        if !self.onsets.is_empty() {
            self.beats += dt / self.period;
        }
        self.quiet = if rms < 0.0005 { self.quiet + dt } else { 0. };
        let audible =
            self.quiet < 0.10 && rms > 0.00005 || self.quiet < 0.08 && !self.onsets.is_empty();
        let density = self
            .onsets
            .iter()
            .filter(|time| self.seconds - **time < 2.)
            .count() as f64
            / 6.;
        let target = if audible {
            0.75 * (rms / 0.14).clamp(0., 1.) + 0.25 * density.min(1.)
        } else {
            0.
        };
        let response = 1. - (-dt / if target > self.energy { 0.12 } else { 0.6 }).exp();
        self.energy += (target - self.energy) * response;
        if self.quiet >= 2. {
            self.onsets.clear();
            self.flux.clear();
            self.beats = 0.;
            self.period = 0.5;
        }
        MusicPulse {
            bpm: 60. / self.period,
            beats: self.beats,
            energy: self.energy,
            onset_strength: flux,
            audible,
            streaming: false,
            locked: self.onsets.len() >= 4,
            sampled_at: Some(Instant::now()),
        }
    }

    fn track_tempo(&mut self) {
        let mut intervals: Vec<f64> = self
            .onsets
            .iter()
            .zip(self.onsets.iter().skip(1))
            .map(|(a, b)| b - a)
            .filter(|gap| (0.3..=1.01).contains(gap))
            .collect();
        if intervals.len() < 3 {
            return;
        }
        intervals.sort_by(f64::total_cmp);
        let hint = intervals[intervals.len() / 2];
        let history: Vec<_> = self.flux.iter().copied().collect();
        let mut best = (f64::NEG_INFINITY, (hint * 100.).round() as usize);
        // Lag correlation runs four times a second over twelve seconds.
        // Prefer the observed inter-onset period to avoid octave jumps.
        for lag in 30..=100 {
            let mut cross = 0.;
            let mut left = 0.;
            let mut right = 0.;
            for index in lag..history.len() {
                cross += history[index] * history[index - lag];
                left += history[index] * history[index];
                right += history[index - lag] * history[index - lag];
            }
            let correlation = cross / (left * right).sqrt().max(1e-12);
            let proximity = (-((lag as f64 / 100. - hint) / 0.08).powi(2)).exp();
            let score = correlation * 0.75 + proximity * 0.25;
            if score > best.0 {
                best = (score, lag);
            }
        }
        let candidate = if (best.1 as f64 / 100. - hint).abs() < 0.025 {
            hint
        } else {
            best.1 as f64 / 100.
        };
        // Change tempo without resetting the continuous beat/bar counter.
        self.period += (candidate - self.period) * 0.35;
    }
}

pub fn offsets(beats: f64, energy: f64, seed: u64) -> crate::JointPositions {
    let variation = (beats / 19.3 + seed as f64 % 17.).sin();
    let sway = (std::f64::consts::TAU * beats / 8.).sin();
    let scale = energy.clamp(0., 1.);
    let turn = 0.025 * scale * (0.8 + 0.2 * variation) * sway;
    let beat = beats.round() as u64;
    let hash = beat.wrapping_add(seed).wrapping_mul(0x9e3779b97f4a7c15);
    let bob = if energy >= 0.55 && hash.rotate_left(13) % 5 != 0 {
        (std::f64::consts::TAU * beats).cos().max(0.).powi(2) * 0.045 * scale
    } else {
        0.
    };
    [
        ("base_yaw_joint", turn),
        ("head_roll_joint", -turn * 0.8),
        ("head_pitch_joint", bob - 0.015 * scale * sway),
        ("shoulder_pitch_joint", 0.005 * scale * sway),
        ("elbow_pitch_joint", -0.008 * scale * sway),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value))
    .collect()
}

pub fn performance(pulse: MusicPulse, seed: u64) -> (crate::MotionDefinition, f64) {
    use crate::{KeyframeArrival, MotionDefinition, MotionKeyframe, MotionSpace, MotionStyle};
    let span = if pulse.energy >= 0.55 { 4. } else { 8. };
    let end = (pulse.beats / span).floor() * span + span;
    let mut previous = pulse.beats;
    let mut next = (pulse.beats * 4.).floor() / 4. + 0.25;
    let mut keyframes = vec![];
    while next <= end + 1e-9 {
        // The audio is queued ahead of the speaker; the head leads by the
        // existing speech stroke interval to account for its physical response.
        let phase = next + (0.12 - 0.04) * pulse.bpm / 60.;
        keyframes.push(MotionKeyframe {
            pose_name: None,
            target: offsets(phase, pulse.energy, seed),
            duration_seconds: (next - previous) * 60. / pulse.bpm,
            arrival: KeyframeArrival::Through,
            hold_seconds: 0.,
            marker: None,
        });
        previous = next;
        next += 0.25;
    }
    keyframes.push(MotionKeyframe {
        pose_name: None,
        target: crate::JointPositions::new(),
        duration_seconds: 0.45,
        arrival: KeyframeArrival::Settle,
        hold_seconds: 0.,
        marker: Some("music_still".into()),
    });
    let style = MotionStyle {
        tempo: 1.,
        amplitude: 1.,
        settle_character: 0.5,
        ..MotionStyle::named("living_idle").expect("idle style")
    };
    (
        MotionDefinition {
            name: "music_performance".into(),
            description: "Sway gently and nod with the music, then return to the held pose.".into(),
            space: MotionSpace::AnchorRelative,
            style,
            return_to_anchor: true,
            keyframes,
        },
        end - 0.5,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn click_tracks_establish_tempo_and_silence_has_no_energy_or_beat_lock() {
        for bpm in [60., 90., 120., 150., 180.] {
            let mut tracker = BeatTracker::default();
            let mut pulse = MusicPulse::default();
            for block in 0..1600 {
                let pcm: Vec<_> = (0..480)
                    .map(|index| {
                        let time = (block * 480 + index) as f64 / 48_000.;
                        let phase = time % (60. / bpm);
                        let value = if phase < 0.012 {
                            (1. - phase / 0.012) as f32 * 0.5
                        } else {
                            0.
                        };
                        [value; 2]
                    })
                    .collect();
                pulse = tracker.process(&pcm);
            }
            assert!(pulse.locked, "{bpm}");
            assert!(
                (pulse.bpm - bpm).abs() < 2.,
                "wanted {bpm}, got {}",
                pulse.bpm
            );
            for _ in 0..400 {
                pulse = tracker.process(&[[0.; 2]; 480]);
            }
            assert!(!pulse.audible && !pulse.locked);
            assert!(pulse.energy < 0.005);
        }
    }
    #[test]
    fn energy_mapping_is_small_varied_and_motionless_at_zero() {
        for beat in 0..320 {
            let t = beat as f64 / 8.;
            let calm = offsets(t, 0.25, 7);
            let upbeat = offsets(t, 0.9, 7);
            for (name, value) in upbeat {
                assert!(
                    value.abs()
                        <= if name == "head_pitch_joint" {
                            0.06
                        } else {
                            0.03
                        }
                );
            }
            assert!(calm.values().all(|value| value.abs() < 0.02));
            assert!(offsets(t, 0., 7).values().all(|value| *value == 0.));
        }
        assert_ne!(offsets(1., 0.9, 7), offsets(9., 0.9, 7));
        let (motion, _) = performance(
            MusicPulse {
                bpm: 120.,
                beats: 8.,
                energy: 0.8,
                ..MusicPulse::default()
            },
            7,
        );
        assert!(
            (motion
                .keyframes
                .iter()
                .map(|frame| frame.duration_seconds)
                .sum::<f64>()
                - 2.45)
                .abs()
                < 1e-9
        );
        assert_eq!(motion.style.tempo, 1.);
        assert_eq!(
            motion.keyframes.last().unwrap().arrival,
            crate::KeyframeArrival::Settle
        );
    }
}
