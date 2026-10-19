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

impl MusicPulse {
    pub fn fresh(self) -> bool {
        self.sampled_at
            .is_some_and(|at| at.elapsed().as_secs_f64() <= 0.5)
    }
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

fn ease(value: f64) -> f64 {
    let t = value.clamp(0., 1.);
    t * t * t * (10. + t * (-15. + 6. * t))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motif {
    Gentle,
    Bounce,
    SideGroove,
    LookAround,
    Finish,
}

pub fn motif(phrase: u64, energy: f64, seed: u64) -> Motif {
    let choice = phrase
        .wrapping_add(seed)
        .wrapping_mul(0x9e3779b97f4a7c15)
        .rotate_left(17);
    if energy < 0.28 {
        Motif::Gentle
    } else if energy < 0.55 {
        if choice % 2 == 0 {
            Motif::Bounce
        } else {
            Motif::SideGroove
        }
    } else {
        [
            Motif::Bounce,
            Motif::SideGroove,
            Motif::LookAround,
            Motif::Finish,
        ][choice as usize % 4]
    }
}

// One full sway lasts two bars. Each reversal has a small outward preparation;
// only the neck overshoots the destination and then settles there.
fn turn(beats: f64, overshoot: f64) -> f64 {
    let phase = beats.rem_euclid(8.);
    let (time, direction) = if phase < 4. {
        (phase, 1.)
    } else {
        (phase - 4., -1.)
    };
    let knots = [
        (0., -1.),
        (0.35, -1.035),
        (2.8, 1. + overshoot),
        (3.45, 1.),
        (4., 1.),
    ];
    let pair = knots.windows(2).find(|pair| time <= pair[1].0).unwrap();
    direction
        * (pair[0].1 + (pair[1].1 - pair[0].1) * ease((time - pair[0].0) / (pair[1].0 - pair[0].0)))
}

fn body_beat(beats: f64) -> f64 {
    let phase = (beats + 0.18).rem_euclid(1.);
    if phase < 0.18 {
        ease(phase / 0.18)
    } else {
        1. - ease((phase - 0.18) / 0.82)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DanceShape {
    pub arm: [f64; 2],
    pub lift_between: bool,
    pub head: f64,
}
impl Default for DanceShape {
    fn default() -> Self {
        Self {
            arm: [0.14, -0.11],
            lift_between: true,
            head: 1.,
        }
    }
}

fn drawing(beats: f64, energy: f64, bpm: f64, kind: Motif, shape: DanceShape) -> [f64; 5] {
    use std::f64::consts::{PI, TAU};
    let e = energy.clamp(0., 1.);
    let drive = e.powf(0.65);
    let (sway, bounce, glance, accent) = match kind {
        Motif::Gentle => (0.14, 0.12, 0.025, 0.),
        Motif::Bounce => (0.27, 1., 0., 0.10),
        Motif::SideGroove => (0.33, 0.85, 0.035, 0.12),
        Motif::LookAround => (0.29, 0.72, 0.09, 0.09),
        Motif::Finish => (0.35, 1., 0.045, 0.14),
    };
    // This arm direction raises the V2 head. Recover upward between beats,
    // then return quickly to the lower held pose on the beat.
    let body = if shape.lift_between {
        1. - body_beat(beats)
    } else {
        body_beat(beats)
    };
    let lift = body * (e / 0.8).clamp(0., 1.).powf(0.8) * bounce;
    let breathing = (0.5 + 0.5 * (TAU * beats / 8.).sin()) * 0.014 * drive;
    let strong = (0.5 + 0.5 * (TAU * beats / 4.).cos()).powi(12);
    let phrase = beats.rem_euclid(16.);
    let finish = if kind == Motif::Finish && phrase >= 12. {
        (PI * (phrase - 12.) / 4.).sin().powi(2) * drive
    } else {
        0.
    };
    // Positive V2 wrist pitch looks down. The body pulse carries the head on
    // every beat; wrist accents occur only on selected strong beats.
    [
        sway * drive * turn(beats, 0.),
        0.19 * drive * turn(beats + bpm * 0.2 / 60., 0.12)
            + glance * drive * (TAU * beats / 16.).sin(),
        shape.head * (accent * drive * strong + finish * 0.07 + breathing * 0.6),
        shape.arm[0] * (lift + breathing / 0.14 + finish * 0.025 / 0.14),
        shape.arm[1] * (lift + breathing / 0.14 + finish * 0.025 / 0.14),
    ]
}

fn offsets_from(
    beats: f64,
    energy: f64,
    previous_energy: f64,
    bpm: f64,
    seed: u64,
    shape: DanceShape,
) -> crate::JointPositions {
    let phrase = (beats / 16.).floor().max(0.) as u64;
    let blend = ease(beats.rem_euclid(16.) / 4.);
    let previous = drawing(
        beats,
        previous_energy,
        bpm,
        motif(phrase.saturating_sub(1), previous_energy, seed),
        shape,
    );
    let current = drawing(beats, energy, bpm, motif(phrase, energy, seed), shape);
    [
        "base_yaw_joint",
        "head_roll_joint",
        "head_pitch_joint",
        "shoulder_pitch_joint",
        "elbow_pitch_joint",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
        (
            name.into(),
            previous[i] + (current[i] - previous[i]) * blend,
        )
    })
    .collect()
}

pub fn offsets(beats: f64, energy: f64, seed: u64) -> crate::JointPositions {
    offsets_from(beats, energy, energy, 120., seed, DanceShape::default())
}

fn tempo(pulse: MusicPulse) -> f64 {
    if pulse.bpm.is_finite() && pulse.bpm > 0. {
        pulse.bpm
    } else {
        120.
    }
}

fn definition(frames: Vec<crate::MotionKeyframe>, name: &str) -> crate::MotionDefinition {
    crate::MotionDefinition {
        name: name.into(),
        description: if name == "music_release" {
            "Return to the held pose with a small final neck movement."
        } else {
            "Bounce with the beat, sway from side to side, then settle at the held pose."
        }
        .into(),
        space: crate::MotionSpace::AnchorRelative,
        return_to_anchor: true,
        // Timing and overlap are drawn into the targets, rather than added by
        // an idle style after the beat clock has scheduled them.
        style: crate::MotionStyle {
            tempo: 1.,
            amplitude: 1.,
            joint_lag: 0.,
            overshoot_scale: 0.,
            tangent_tension: 0.6,
            settle_character: 0.5,
            ..crate::MotionStyle::named("living_idle").unwrap()
        },
        keyframes: frames,
    }
}

pub fn release(pulse: MusicPulse, offsets: &crate::JointPositions) -> crate::MotionDefinition {
    let beat = 60. / tempo(pulse);
    let mut previous = 0.;
    let frames = [(0.28, 0.62), (0.8, -0.055), (1., 0.)]
        .into_iter()
        .map(|(at, gain)| {
            let target = offsets
                .iter()
                .map(|(name, offset)| {
                    let gain = if gain < 0.
                        && !matches!(name.as_str(), "base_yaw_joint" | "head_roll_joint")
                    {
                        0.035
                    } else {
                        gain
                    };
                    (name.clone(), offset * gain)
                })
                .collect();
            let frame = crate::MotionKeyframe {
                pose_name: None,
                target,
                duration_seconds: (at - previous) * beat,
                arrival: if at == 1. {
                    crate::KeyframeArrival::Settle
                } else {
                    crate::KeyframeArrival::Through
                },
                hold_seconds: 0.,
                marker: if at == 1. {
                    Some("music_still".into())
                } else {
                    None
                },
            };
            previous = at;
            frame
        })
        .collect();
    definition(frames, "music_release")
}

pub fn plan(
    pulse: MusicPulse,
    seed: u64,
    previous_energy: f64,
    entrance: Option<&crate::JointPositions>,
) -> (crate::MotionDefinition, f64) {
    plan_with_shape(
        pulse,
        seed,
        previous_energy,
        entrance,
        DanceShape::default(),
    )
}

pub(crate) fn plan_with_shape(
    pulse: MusicPulse,
    seed: u64,
    previous_energy: f64,
    entrance: Option<&crate::JointPositions>,
    shape: DanceShape,
) -> (crate::MotionDefinition, f64) {
    let bpm = tempo(pulse);
    let boundary = (pulse.beats / 16.).floor() * 16. + 16.;
    let extending = entrance.is_none() && boundary - pulse.beats <= 1.;
    let end = boundary + if extending { 16. } else { 0. };
    let mut times = Vec::new();
    let mut next = (pulse.beats * 8.).floor() / 8. + 0.125;
    while next <= end + 1e-8 {
        times.push(next);
        next += 0.125;
    }
    // Draw the short compression exactly; a quarter-beat grid would miss it.
    for beat in pulse.beats.floor() as i64..=end as i64 {
        let at = beat as f64 + 0.82;
        if at > pulse.beats + 1e-7 && at < end {
            times.push(at);
        }
    }
    if entrance.is_some() && pulse.beats + 1. < end {
        times.push(pulse.beats + 1.);
    }
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
    let mut previous = pulse.beats;
    let mut frames = Vec::new();
    for at in times {
        let energy = if extending && at < boundary {
            previous_energy
        } else {
            pulse.energy
        };
        let mut target = offsets_from(at, energy, previous_energy, bpm, seed, shape);
        if let Some(start) = entrance {
            let weight = ease(at - pulse.beats);
            for (name, value) in &mut target {
                *value = start.get(name).copied().unwrap_or(0.) * (1. - weight) + *value * weight;
            }
        }
        frames.push(crate::MotionKeyframe {
            pose_name: None,
            target,
            duration_seconds: (at - previous) * 60. / bpm,
            arrival: crate::KeyframeArrival::Through,
            hold_seconds: 0.,
            marker: if entrance.is_some() && (at - pulse.beats - 1.).abs() < 1e-7 {
                Some("music_entered".into())
            } else {
                None
            },
        });
        previous = at;
    }
    let end_offset = frames.last().map(|f| f.target.clone()).unwrap_or_default();
    frames.extend(release(pulse, &end_offset).keyframes);
    (definition(frames, "music_performance"), end - 0.5)
}

pub fn performance(pulse: MusicPulse, seed: u64) -> (crate::MotionDefinition, f64) {
    plan(
        pulse,
        seed,
        pulse.energy,
        Some(&crate::JointPositions::new()),
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
    fn body_marks_every_beat_and_wrist_marks_only_strong_beats() {
        for beat in 32..40 {
            let down = offsets(beat as f64, 0.8, 7);
            let up = offsets(beat as f64 + 0.8, 0.8, 7);
            assert!(up["shoulder_pitch_joint"] - down["shoulder_pitch_joint"] > 0.06);
            assert!(up["elbow_pitch_joint"] - down["elbow_pitch_joint"] < -0.04);
            if beat % 4 == 0 {
                assert!(down["head_pitch_joint"] > 0.05);
            } else {
                assert!(down["head_pitch_joint"].abs() < 0.04);
            }
        }
    }
    #[test]
    fn neck_leads_the_base_and_all_five_motifs_are_available() {
        let mut kinds = std::collections::BTreeSet::new();
        for phrase in 0..16 {
            for energy in [0.1, 0.4, 0.9] {
                kinds.insert(format!("{:?}", motif(phrase, energy, 7)));
            }
        }
        assert_eq!(kinds.len(), 5);
        for bpm in [80., 110., 128.] {
            let crossing = |joint: usize| {
                (1..3000)
                    .map(|i| i as f64 / 1000.)
                    .find(|beat| {
                        drawing(*beat, 1., bpm, Motif::Bounce, DanceShape::default())[joint] >= 0.
                    })
                    .unwrap()
            };
            let lead = (crossing(0) - crossing(1)) * 60. / bpm;
            assert!((lead - 0.2).abs() < 0.06, "{bpm}: neck lead {lead}");
        }
    }

    #[test]
    fn motifs_blend_without_sign_flips_and_starts_take_a_full_beat() {
        let pulse = MusicPulse {
            bpm: 128.,
            beats: 16.12,
            energy: 0.9,
            ..MusicPulse::default()
        };
        let start = offsets(10., 0.4, 8);
        let (motion, _) = plan(pulse, 7, 0.2, Some(&start));
        let mut time = 0.;
        let mut entered = 0.;
        for frame in &motion.keyframes {
            time += frame.duration_seconds;
            if frame.marker.as_deref() == Some("music_entered") {
                entered = time;
            }
        }
        assert!((entered - 60. / 128.).abs() < 1e-8);
        let first = &motion.keyframes[0];
        for (joint, value) in &first.target {
            assert!((value - start[joint]).abs() < 0.002);
        }
        for boundary in [16., 32., 48.] {
            for joint in ["base_yaw_joint", "head_roll_joint"] {
                let before = offsets(boundary - 1e-5, 0.9, 7)[joint];
                let after = offsets(boundary + 1e-5, 0.9, 7)[joint];
                assert!((after - before).abs() < 1e-4);
            }
        }
        assert!(offsets(5., 0., 7).values().all(|value| *value == 0.));
        assert!(
            motion.keyframes[..motion.keyframes.len() - 1]
                .iter()
                .all(|f| f.arrival == crate::KeyframeArrival::Through)
        );
    }
}
