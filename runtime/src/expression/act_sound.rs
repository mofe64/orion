//! Rare act sounds follow compiled markers and never control movement.
use crate::{AudioDevice, StateSnapshot};

#[derive(Clone, Copy)]
pub(crate) struct SoundContext {
    pub enabled: bool,
    pub speech: bool,
    pub alert: bool,
    pub quiet_routine: bool,
    pub hour: f64,
}
impl SoundContext {
    fn allowed(self) -> bool {
        self.enabled
            && !self.speech
            && !self.alert
            && !self.quiet_routine
            && (7. ..22.).contains(&self.hour)
    }
}
#[derive(Default)]
pub(crate) struct ActSounds {
    last_run: Option<u64>,
    consumed: bool,
}
impl ActSounds {
    pub fn tick(
        &mut self,
        snapshot: &StateSnapshot,
        context: SoundContext,
        audio: &mut dyn AudioDevice,
    ) {
        let Some(motion) = snapshot.motion.as_ref().or(snapshot.last_motion.as_ref()) else {
            return;
        };
        if self.last_run != Some(motion.run_id) {
            self.last_run = Some(motion.run_id);
            self.consumed = false;
        }
        if self.consumed {
            return;
        }
        let cue = match motion.name.as_str() {
            "wake_stretch" => "wake_chirp",
            "sleep_settle" => "sleep_sigh",
            "yawn" => "yawn_hum",
            _ => return,
        };
        if !motion
            .reached_markers
            .iter()
            .any(|marker| marker == &motion.name)
        {
            return;
        }
        self.consumed = true;
        if context.allowed() {
            let _ = audio.play(cue);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioCommand, MotionState, MovementPhase, RecordingAudioDevice, RuntimeMode};
    fn snapshot(marker: bool) -> StateSnapshot {
        StateSnapshot {
            schema_version: 1,
            robot: "orion",
            build_revision: "test",
            mode: RuntimeMode::Holding,
            profile_applied: true,
            torque_enabled: true,
            sequence: 0,
            sampled_at_unix_ns: 0,
            update_hz: 50.,
            bus_error_count: 0,
            bus_fault: None,
            joints: vec![],
            last_motion: None,
            motion: Some(MotionState {
                run_id: 1,
                name: "wake_stretch".into(),
                state: MovementPhase::Executing,
                keyframe: None,
                keyframe_index: None,
                keyframe_count: None,
                reached_markers: if marker {
                    vec!["wake_stretch".into()]
                } else {
                    vec![]
                },
                progress: 0.5,
                max_position_error_rad: None,
                max_velocity_rad_s: None,
                completion_diagnostic: None,
                error: None,
            }),
        }
    }
    fn context() -> SoundContext {
        SoundContext {
            enabled: true,
            speech: false,
            alert: false,
            quiet_routine: false,
            hour: 12.,
        }
    }
    #[test]
    fn marker_fires_once_and_each_block_consumes_it() {
        let mut player = ActSounds::default();
        let mut audio = RecordingAudioDevice::default();
        player.tick(&snapshot(false), context(), &mut audio);
        assert!(audio.commands().is_empty());
        player.tick(&snapshot(true), context(), &mut audio);
        player.tick(&snapshot(true), context(), &mut audio);
        assert_eq!(audio.commands(), &[AudioCommand::Play("wake_chirp".into())]);
        for blocked in [
            SoundContext {
                enabled: false,
                ..context()
            },
            SoundContext {
                speech: true,
                ..context()
            },
            SoundContext {
                alert: true,
                ..context()
            },
            SoundContext {
                quiet_routine: true,
                ..context()
            },
            SoundContext {
                hour: 22.,
                ..context()
            },
            SoundContext {
                hour: 6.99,
                ..context()
            },
        ] {
            let mut player = ActSounds::default();
            let mut audio = RecordingAudioDevice::default();
            player.tick(&snapshot(true), blocked, &mut audio);
            player.tick(&snapshot(true), context(), &mut audio);
            assert!(audio.commands().is_empty());
        }
    }
    #[test]
    fn unavailable_audio_does_not_change_motion() {
        let mut player = ActSounds::default();
        let mut audio = crate::UnavailableAudioDevice;
        let state = snapshot(true);
        player.tick(&state, context(), &mut audio);
        assert_eq!(
            state.motion.as_ref().unwrap().state,
            MovementPhase::Executing
        );
        assert!(player.consumed);
    }
    #[test]
    fn each_rare_act_uses_its_own_marker_and_cue() {
        for (name, cue) in [
            ("wake_stretch", "wake_chirp"),
            ("sleep_settle", "sleep_sigh"),
            ("yawn", "yawn_hum"),
        ] {
            let mut state = snapshot(true);
            let motion = state.motion.as_mut().unwrap();
            motion.name = name.into();
            motion.reached_markers = vec![name.into()];
            let mut player = ActSounds::default();
            let mut audio = RecordingAudioDevice::default();
            player.tick(&state, context(), &mut audio);
            assert_eq!(audio.commands(), &[AudioCommand::Play(cue.into())]);
        }
    }
}
