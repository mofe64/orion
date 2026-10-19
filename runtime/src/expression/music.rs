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
    tempo_flux: VecDeque<f64>,
    low_flux: VecDeque<f64>,
    tempo_onsets: VecDeque<f64>,
    tempo_tail: [f64; 4],
    tempo_mean: f64,
    tempo_target: f64,
    tempo_locked: bool,
    pending_tempo: Option<(f64, f64)>,
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
            tempo_flux: VecDeque::new(),
            low_flux: VecDeque::new(),
            tempo_onsets: VecDeque::new(),
            tempo_tail: [0.; 4],
            tempo_mean: 0.,
            tempo_target: 0.5,
            tempo_locked: false,
            pending_tempo: None,
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
        let mut changes = [0.; 3];
        for (index, band) in bands.into_iter().enumerate() {
            let level = (band / count).sqrt();
            changes[index] = (level - self.previous[index]).max(0.);
            flux += changes[index];
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
        // Tempo has its own kick-led evidence. Keep the original onset stream
        // for energy, audibility and phase so the dance's energy input is intact.
        let weighted = changes[0] * 4. + changes[1] * 0.5 + changes[2] * 0.125;
        let tempo_onset = weighted > (self.tempo_mean * 2.).max(0.0001)
            && rms > 0.001
            && self
                .tempo_onsets
                .back()
                .is_none_or(|last| self.seconds - last > 0.22);
        if tempo_onset {
            self.tempo_onsets.push_back(self.seconds);
        }
        self.tempo_mean += (weighted - self.tempo_mean) * 0.01;
        while self
            .tempo_onsets
            .front()
            .is_some_and(|time| self.seconds - time > 12.)
        {
            self.tempo_onsets.pop_front();
        }
        // Compression tolerates unequal drum strength; a short triangular
        // window tolerates a few milliseconds of onset timing variation.
        let root = weighted.powf(0.4);
        let smooth = (root
            + 2. * self.tempo_tail[0]
            + 3. * self.tempo_tail[1]
            + 2. * self.tempo_tail[2]
            + self.tempo_tail[3])
            / 9.;
        self.tempo_tail.rotate_right(1);
        self.tempo_tail[0] = root;
        self.flux.push_back(flux);
        self.tempo_flux.push_back(smooth);
        self.low_flux.push_back(changes[0]);
        if self.flux.len() > 1200 {
            self.flux.pop_front();
            self.tempo_flux.pop_front();
            self.low_flux.pop_front();
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
            self.tempo_flux.clear();
            self.low_flux.clear();
            self.tempo_onsets.clear();
            self.tempo_tail = [0.; 4];
            self.tempo_mean = 0.;
            self.tempo_target = 0.5;
            self.tempo_locked = false;
            self.pending_tempo = None;
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
            locked: self.tempo_locked && self.onsets.len() >= 4,
            sampled_at: Some(Instant::now()),
        }
    }

    fn track_tempo(&mut self) {
        let onsets = if self.tempo_onsets.len() >= 4 {
            &self.tempo_onsets
        } else {
            &self.onsets
        };
        let mut intervals: Vec<f64> = onsets
            .iter()
            .zip(onsets.iter().skip(1))
            .map(|(a, b)| b - a)
            .filter(|gap| (0.3..=1.01).contains(gap))
            .collect();
        if intervals.len() < 3 {
            return;
        }
        intervals.sort_by(f64::total_cmp);
        let hint = intervals[intervals.len() / 2];
        let history: Vec<_> = self.tempo_flux.iter().copied().collect();
        let mut correlations = [0_f64; 101];
        let mut best = (f64::NEG_INFINITY, 50);
        // Same lag range, window and update cadence as the original tracker.
        for lag in 30..=100 {
            correlations[lag] = correlation(&history, lag);
            let proximity = (-((lag as f64 / 100. - hint) / 0.08).powi(2)).exp();
            let score = correlations[lag] * 0.75 + proximity * 0.25;
            if score > best.0 {
                best = (score, lag);
            }
        }
        let peak = |at: f64| {
            let centre = at.round() as usize;
            (centre.saturating_sub(2).max(30)..=(centre + 2).min(100))
                .max_by(|a, b| correlations[*a].total_cmp(&correlations[*b]))
                .unwrap()
        };
        let base = peak(best.1 as f64);
        let mut selected = base;
        let mut selected_preference = tempo_preference(60. / refined_period(&correlations, base));
        let base_support = self.low_alignment(refined_period(&correlations, base));
        let base_evidence = correlations[base] * (0.75 + 0.25 * (base_support / 3.).min(1.));
        // A syncopated kick can suggest a three-eighth-note gap. The original
        // attacks nominate a candidate too, subject to the same bass support.
        let mut raw_intervals: Vec<_> = self
            .onsets
            .iter()
            .zip(self.onsets.iter().skip(1))
            .map(|(a, b)| b - a)
            .filter(|gap| (0.3..=1.01).contains(gap))
            .collect();
        raw_intervals.sort_by(f64::total_cmp);
        let raw_hint = raw_intervals
            .get(raw_intervals.len() / 2)
            .copied()
            .unwrap_or(hint);
        for suggestion in [base as f64 * 2., base as f64 / 2., raw_hint * 100.] {
            if !(30. ..=100.).contains(&suggestion) {
                continue;
            }
            let lag = peak(suggestion);
            let period = refined_period(&correlations, lag);
            let support = self.low_alignment(period);
            let evidence = correlations[lag] * (0.75 + 0.25 * (support / 3.).min(1.));
            let preference = tempo_preference(60. / period);
            if evidence >= base_evidence * 0.8 && support >= 1.5 && preference > selected_preference
            {
                selected = lag;
                selected_preference = preference;
            }
        }
        // Refine only the chosen peak against the uncompressed attacks. This
        // retains sub-frame tempo precision without another complete lag scan.
        let raw: Vec<_> = self.flux.iter().copied().collect();
        let mut local = [0_f64; 101];
        for lag in selected.saturating_sub(2).max(30)..=(selected + 2).min(100) {
            local[lag] = correlation(&raw, lag);
        }
        let refined = (selected.saturating_sub(1).max(30)..=(selected + 1).min(100))
            .max_by(|a, b| local[*a].total_cmp(&local[*b]))
            .unwrap();
        let mut candidate = refined_period(&local, refined);
        // Average nearby observed gaps to remove the 10 ms frame rounding
        // bias when the attacks already agree with the selected beat period.
        let nearby: Vec<_> = raw_intervals
            .iter()
            .copied()
            .filter(|gap| (gap - raw_hint).abs() <= 0.03)
            .collect();
        if nearby.len() >= 4 {
            let observed = nearby.iter().sum::<f64>() / nearby.len() as f64;
            if (observed / candidate).ln().abs() < 0.08 {
                candidate = observed;
            }
        }
        let incumbent = peak(self.tempo_target * 100.);
        let wins = selected_preference > tempo_preference(60. / self.tempo_target) * 1.08
            || correlations[incumbent] < correlations[selected] * 0.75;
        self.follow_tempo(candidate, wins);
    }

    fn follow_tempo(&mut self, candidate: f64, wins: bool) {
        let close = (candidate / self.tempo_target).ln().abs() < 0.12;
        if !self.tempo_locked || (!close && wins) {
            let pending = self.pending_tempo.get_or_insert((candidate, self.seconds));
            if (candidate / pending.0).ln().abs() > 0.06 {
                *pending = (candidate, self.seconds);
            }
            if !self.tempo_locked {
                self.tempo_target = candidate;
            }
            let required = if self.tempo_locked { 3. } else { 1.5 };
            if self.seconds - pending.1 >= required {
                self.tempo_target = candidate;
                self.tempo_locked = true;
                self.pending_tempo = None;
            }
        } else {
            self.pending_tempo = None;
            if close {
                self.tempo_target = candidate;
            }
        }
        // Beat/bar phase stays continuous, including an accepted octave change.
        self.period += (self.tempo_target - self.period) * 0.35;
    }

    fn low_alignment(&self, period: f64) -> f64 {
        let total = self.low_flux.iter().sum::<f64>();
        if total < 1e-9 || total < self.flux.iter().sum::<f64>() * 0.12 {
            return 0.;
        }
        let bins = (period * 100.).round() as usize;
        let mut phases = [0_f64; 101];
        for (i, onset) in self.low_flux.iter().enumerate() {
            let phase = (i as f64 / (period * 100.) * bins as f64).round() as usize % bins;
            phases[phase] += onset;
        }
        let peak = (0..bins)
            .map(|i| (0..7).map(|j| phases[(i + j) % bins]).sum::<f64>())
            .fold(0_f64, f64::max);
        peak / total / (7. / bins as f64)
    }
}

fn correlation(history: &[f64], lag: usize) -> f64 {
    let (mut cross, mut left, mut right) = (0., 0., 0.);
    for i in lag..history.len() {
        cross += history[i] * history[i - lag];
        left += history[i] * history[i];
        right += history[i - lag] * history[i - lag];
    }
    cross / (left * right).sqrt().max(1e-12)
}

fn refined_period(correlations: &[f64; 101], lag: usize) -> f64 {
    let offset = if (31..100).contains(&lag) {
        let [left, centre, right] = [
            correlations[lag - 1],
            correlations[lag],
            correlations[lag + 1],
        ];
        let curvature = left - 2. * centre + right;
        if curvature < -1e-9 {
            (0.5 * (left - right) / curvature).clamp(-0.5, 0.5)
        } else {
            0.
        }
    } else {
        0.
    };
    (lag as f64 + offset) / 100.
}

fn tempo_preference(bpm: f64) -> f64 {
    (-0.5 * ((bpm / 108.).ln() / 0.4).powi(2)).exp()
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
        for (bpm, expected) in [
            (60., 60.),
            (90., 90.),
            (120., 120.),
            (150., 150.),
            (180., 90.),
        ] {
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
                (pulse.bpm - expected).abs() < 2.,
                "input {bpm}, wanted {expected}, got {}",
                pulse.bpm
            );
            for _ in 0..400 {
                pulse = tracker.process(&[[0.; 2]; 480]);
            }
            assert!(!pulse.audible && !pulse.locked);
            assert!(pulse.energy < 0.005);
        }
    }
    fn drums(bpm: f64, broken: bool, seconds: usize) -> Vec<MusicPulse> {
        use std::f64::consts::TAU;
        let mut tracker = BeatTracker::default();
        (0..seconds * 100)
            .map(|block| {
                let pcm: Vec<_> = (0..480)
                    .map(|i| {
                        let time = (block * 480 + i) as f64 / 48_000.;
                        let beats = time * bpm / 60.;
                        let beat = beats.floor() as usize;
                        let kick_phase = if broken {
                            let bar = beats.rem_euclid(4.);
                            if bar >= 2.5 { bar - 2.5 } else { bar }
                        } else {
                            beats.fract()
                        };
                        let kick_time = kick_phase * 60. / bpm;
                        let kick = 0.7 * (TAU * 62. * kick_time).sin() * (-kick_time * 32.).exp();
                        let hat_time = (beats * 2.).fract() * 30. / bpm;
                        let hats = 0.45 * (TAU * 7100. * time).sin() * (-hat_time * 150.).exp();
                        let snare_time = beats.fract() * 60. / bpm;
                        let snare = if broken && beat % 2 == 1 {
                            (0.25 * (TAU * 220. * time).sin() + 0.15 * (TAU * 2800. * time).sin())
                                * (-snare_time * 45.).exp()
                        } else {
                            0.
                        };
                        [(kick + hats + snare) as f32; 2]
                    })
                    .collect();
                tracker.process(&pcm)
            })
            .collect()
    }

    #[test]
    fn slow_kicks_with_eighth_note_hats_keep_the_true_beat() {
        for bpm in [70., 80., 90.] {
            let pulses = drums(bpm, false, 24);
            for p in &pulses[1400..] {
                assert!(p.locked, "{bpm}: no lock");
                assert!((p.bpm - bpm).abs() < 2., "{bpm}: got {}", p.bpm);
            }
        }
    }

    #[test]
    fn four_on_the_floor_keeps_120_and_128_bpm() {
        for bpm in [120., 128.] {
            let pulses = drums(bpm, false, 24);
            for p in &pulses[1400..] {
                assert!(p.locked);
                assert!((p.bpm - bpm).abs() < 2., "{bpm}: got {}", p.bpm);
            }
        }
    }

    #[test]
    fn ambiguous_drum_and_bass_uses_consistent_half_time() {
        for bpm in [160., 165., 170.] {
            let pulses = drums(bpm, true, 30);
            for p in &pulses[1800..] {
                assert!(p.locked);
                assert!((p.bpm - bpm / 2.).abs() < 2., "{bpm}: got {}", p.bpm);
            }
        }
    }

    #[test]
    fn octave_changes_need_sustained_evidence_and_keep_the_beat_counter() {
        let mut tracker = BeatTracker {
            tempo_locked: true,
            tempo_target: 0.375,
            period: 0.375,
            beats: 57.25,
            ..Default::default()
        };
        for i in 0..12 {
            tracker.seconds = i as f64 * 0.25;
            tracker.follow_tempo(0.75, true);
            assert_eq!(tracker.tempo_target, 0.375);
            assert_eq!(tracker.beats, 57.25);
        }
        tracker.follow_tempo(0.75, false);
        assert!(tracker.pending_tempo.is_none());
        for i in 12..=24 {
            tracker.seconds = i as f64 * 0.25;
            tracker.follow_tempo(0.75, true);
            if i < 24 {
                assert_eq!(tracker.tempo_target, 0.375);
            }
            assert_eq!(tracker.beats, 57.25);
        }
        assert_eq!(tracker.tempo_target, 0.75);
        assert!(tracker.period > 0.375 && tracker.period < 0.75);
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
