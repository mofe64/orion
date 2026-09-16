//! Alarm assets are embedded in the release; the motor loop only copies PCM.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertSound {
    #[default]
    TwoTone,
    ClubAlarm,
    FunnyAlarm,
}

impl AlertSound {
    pub fn catalog() -> Value {
        json!([
            {"id":Self::TwoTone,"name":"Two-tone (default)"},
            {"id":Self::ClubAlarm,"name":"Club alarm"},
            {"id":Self::FunnyAlarm,"name":"Funny alarm"},
        ])
    }

    fn recording(self) -> Option<&'static [u8]> {
        match self {
            Self::TwoTone => None,
            Self::ClubAlarm => Some(include_bytes!("../../../audio/alarms/club_alarm.pcm")),
            Self::FunnyAlarm => Some(include_bytes!("../../../audio/alarms/funny_alarm.pcm")),
        }
    }

    pub fn pcm(self, start: usize, count: usize) -> Vec<u8> {
        if let Some(recording) = self.recording() {
            // Loop at sample boundaries, including chunks which straddle the end.
            return (start..start + count)
                .flat_map(|sample| {
                    let i = (sample % (recording.len() / 2)) * 2;
                    [recording[i], recording[i + 1]]
                })
                .collect();
        }
        // High/low double pulse with listening gaps, at 24 kHz. A 0.65 peak
        // leaves headroom without changing the mixer or the user's volume.
        (start..start + count)
            .map(|sample| {
                let t = (sample % 28800) as f64 / 24000.;
                let phase = t % 0.6;
                let envelope = if phase < 0.48 {
                    (phase / 0.012).min(1.) * ((0.48 - phase) / 0.012).min(1.)
                } else {
                    0.
                };
                let hz = if t < 0.6 { 880. } else { 1174.66 };
                ((std::f64::consts::TAU * hz * t).sin() * envelope * 0.65 * i16::MAX as f64) as i16
            })
            .flat_map(i16::to_le_bytes)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_recordings_have_headroom_and_loop_without_losing_samples() {
        for sound in [AlertSound::ClubAlarm, AlertSound::FunnyAlarm] {
            let pcm = sound.recording().unwrap();
            assert_eq!(pcm.len() % 2, 0);
            assert!((24000 * 2..=30 * 24000 * 2).contains(&pcm.len()));
            let peak = pcm
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
                .max()
                .unwrap();
            assert!((20000..22000).contains(&peak));
            assert_eq!(&pcm[..2], &[0, 0]);
            assert_eq!(&pcm[pcm.len() - 2..], &[0, 0]);
            let count = pcm.len() / 2;
            assert_eq!(
                sound.pcm(count - 100, 480),
                [&pcm[pcm.len() - 200..], &pcm[..760]].concat()
            );
            assert_eq!(sound.pcm(count * 5, 480), sound.pcm(0, 480));
        }
    }
}
