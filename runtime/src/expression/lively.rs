//! What a Lively Orion does for the people at the desk: it waves back at a
//! raised hand, offers light when the room gets dark, suggests a break after
//! a long stretch of work and says goodnight to people who leave late.
//!
//! Each behaviour reads the shared situation and only acts while a person is
//! in view. Words follow the remark policy, so quiet hours, busy moments and
//! "be quiet" requests apply.
use super::initiative::Act;
use super::reactions::ReactionKind;
use super::surroundings::SeenFace;
use super::*;
use crate::expression::remarks::RemarkEvent;

/// A person must be at the desk this long before Orion suggests a break.
const BREAK_SECONDS: f64 = 90.0 * 60.0;
const DEMO_BREAK_SECONDS: f64 = 60.0;
/// Nobody in view this long ends a stretch at the desk.
const DESK_GAP_SECONDS: f64 = 300.0;
/// The room is dark below this light level, in lux.
const DARK_LUX: f64 = 15.0;
/// Orion's light counts as low below this brightness, in percent.
const LOW_LIGHT_PERCENT: f64 = 25.0;
/// The room must stay dark this long, so a passing shadow is not darkness.
const DARK_SECONDS: f64 = 60.0;
const DEMO_DARK_SECONDS: f64 = 5.0;
/// A hand must stay raised this long before Orion waves back.
const WAVE_RAISE_SECONDS: f64 = 0.6;
/// Goodnight is said to people who leave from this hour until 05:00.
const GOODNIGHT_HOUR: f64 = 21.0;
/// After a goodnight, Orion rests when nobody comes back for this long.
pub(super) const GOODNIGHT_REST_SECONDS: f64 = 120.0;

#[derive(Debug, Default)]
pub(super) struct LivelyState {
    desk_since: Option<f64>,
    last_face: Option<f64>,
    dark_since: Option<f64>,
    raised_since: Option<f64>,
    /// The raised hand was answered; wait until it is lowered.
    waved: bool,
    /// When Orion rests after a goodnight, unless someone comes back.
    pub(super) goodnight_rest: Option<f64>,
}

/// Late evening: a goodbye is a goodnight.
pub(super) fn late(hour: f64) -> bool {
    !(5.0..GOODNIGHT_HOUR).contains(&hour)
}

impl CharacterCoordinator {
    /// Orion's goodnight is due: nobody came back after it.
    pub(crate) fn take_goodnight_rest(&mut self, now: f64) -> bool {
        let lively = &mut self.surroundings.lively;
        if lively.goodnight_rest.is_some_and(|at| now >= at) {
            lively.goodnight_rest = None;
            return true;
        }
        false
    }

    /// What Orion bases its own decisions on, for Studio's Debug page: the
    /// situation, the curiosity budget, the Lively timers and, for each kind
    /// of remark, why it would not be said now.
    pub(crate) fn initiative_status(&self, now: f64) -> serde_json::Value {
        let lively = &self.surroundings.lively;
        let since = |at: Option<f64>| at.map(|at| (now - at).max(0.0));
        let remarks: serde_json::Map<_, _> = [
            RemarkEvent::PersonArrived,
            RemarkEvent::PersonLooking,
            RemarkEvent::NewObject,
            RemarkEvent::LightCare,
            RemarkEvent::BreakNudge,
            RemarkEvent::PersonLeft,
            RemarkEvent::Goodnight,
        ]
        .into_iter()
        .filter_map(|event| {
            let name = serde_json::to_value(event).ok()?.as_str()?.to_owned();
            Some((name, self.remarks.refusal(event, now).into()))
        })
        .collect();
        serde_json::json!({
            "situation": self.surroundings.situation,
            "curiosity": self.surroundings.budget.status(now),
            "quirks": self.acts.settings.quirks,
            "lively": {
                "desk_seconds": since(lively.desk_since),
                "dark_seconds": since(lively.dark_since),
                "goodnight_rest_seconds": lively.goodnight_rest.map(|at| (at - now).max(0.0)),
            },
            "remark_refusals": remarks,
        })
    }

    pub(super) fn observe_lively<D: RuntimeDriver>(
        &mut self,
        faces: &[SeenFace],
        raised_hands: u8,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        let situation = self.surroundings.situation.clone();
        let demo = situation.liveliness.demo();
        let lively = &mut self.surroundings.lively;
        if faces.is_empty() {
            if lively
                .last_face
                .is_none_or(|last| now - last >= DESK_GAP_SECONDS)
            {
                lively.desk_since = None;
            }
        } else {
            lively.desk_since.get_or_insert(now);
            lively.last_face = Some(now);
            lively.goodnight_rest = None;
        }
        let dark = situation.room_lux.is_some_and(|lux| lux < DARK_LUX)
            && (!situation.lamp.on || situation.lamp.brightness < LOW_LIGHT_PERCENT);
        if dark {
            lively.dark_since.get_or_insert(now);
        } else {
            lively.dark_since = None;
        }
        if raised_hands == 0 || faces.is_empty() {
            lively.raised_since = None;
            lively.waved = false;
        } else {
            lively.raised_since.get_or_insert(now);
        }
        let Some(face) = faces.first() else {
            return Ok(());
        };
        if !situation.liveliness.speaks() {
            return Ok(());
        }
        let lively = &self.surroundings.lively;
        if !demo
            && !lively.waved
            && lively
                .raised_since
                .is_some_and(|since| now - since >= WAVE_RAISE_SECONDS)
        {
            self.surroundings.lively.waved = true;
            return self.wave_back(face, now, core);
        }
        let break_due = lively.desk_since.is_some_and(|since| {
            now - since
                >= if demo {
                    DEMO_BREAK_SECONDS
                } else {
                    BREAK_SECONDS
                }
        });
        let dark_due = lively.dark_since.is_some_and(|since| {
            now - since
                >= if demo {
                    DEMO_DARK_SECONDS
                } else {
                    DARK_SECONDS
                }
        });
        let care = if break_due {
            Some((RemarkEvent::BreakNudge, None, "break_nudge"))
        } else if dark_due {
            let detail = if situation.lamp.on {
                format!("light low at {:.0}%", situation.lamp.brightness)
            } else {
                "light off".into()
            };
            Some((RemarkEvent::LightCare, Some(detail), "light_care"))
        } else {
            None
        };
        let Some((event, detail, behaviour)) = care else {
            return Ok(());
        };
        // Care is speech first: without words, the movement means nothing.
        if self.remarks.refusal(event, now).is_some() || !self.may_start_reaction(core) {
            return Ok(());
        }
        let anchor = self
            .status
            .active_anchor
            .clone()
            .ok_or_else(|| Error::InvalidState("Care needs an anchor".into()))?;
        let (motion, clip) = if event == RemarkEvent::BreakNudge {
            // A stretch says "time to stretch" before any word.
            let definition = core.motions().motion("wake_stretch")?.clone();
            core.validate_curiosity_motion(&definition, &anchor, None)?;
            (definition, "wake_stretch")
        } else {
            (
                core.prepare_arrival_motion(&anchor, face.azimuth, face.elevation)?,
                "arrival_notice",
            )
        };
        if event == RemarkEvent::BreakNudge {
            self.surroundings.lively.desk_since = Some(now);
        }
        self.start_act(
            Act {
                kind: ReactionKind::Care,
                key: behaviour.into(),
                motion,
                centre: anchor.clone(),
                anchor,
                track: face.track_id,
                category: None,
                object: None,
                seconds: 8.0,
                behaviour,
                target: Some(format!("face:{}", face.track_id)),
                target_category: Some("person".into()),
                clip,
                remark: Some((event, detail, None)),
            },
            now,
            core,
        )
    }

    fn wave_back<D: RuntimeDriver>(
        &mut self,
        face: &SeenFace,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if !self.surroundings.settings.mutual_gaze {
            return Ok(());
        }
        if !self.may_start_reaction(core) {
            let reason = self.busy_reason();
            self.skip(
                "wave_back",
                Some(format!("face:{}", face.track_id)),
                Some("person".into()),
                reason,
                now,
            );
            return Ok(());
        }
        let anchor = self
            .status
            .active_anchor
            .clone()
            .ok_or_else(|| Error::InvalidState("Wave needs an anchor".into()))?;
        let motion = core.prepare_wave_motion(&anchor, face.azimuth, face.elevation)?;
        self.perform(
            Act {
                kind: ReactionKind::Wave,
                key: "hand:wave".into(),
                motion,
                centre: anchor.clone(),
                anchor,
                track: face.track_id,
                category: None,
                object: None,
                seconds: 8.0,
                behaviour: "wave_back",
                target: Some(format!("face:{}", face.track_id)),
                target_category: Some("person".into()),
                clip: "wave_back",
                remark: None,
            },
            now,
            core,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::situation::{Conditions, LampView};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn goodnight_starts_at_nine_and_ends_at_five() {
        assert!(!late(20.9));
        assert!(late(21.0) && late(23.5) && late(0.5) && late(4.9));
        assert!(!late(5.0) && !late(12.0));
    }

    struct Desk {
        core: RuntimeCore<crate::expression::gaze::tests::FollowingDriver>,
        boot: std::sync::Arc<AtomicU64>,
        character: CharacterCoordinator,
        azimuth: f64,
        elevation: f64,
    }
    impl Desk {
        fn new(hour: f64) -> Self {
            let (mut core, _, boot) = crate::expression::gaze::tests::following();
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap();
            let profile: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(root.join("hardware/profiles/v2.json")).unwrap(),
            )
            .unwrap();
            core.configure_comfort_box(
                serde_json::from_value(profile["comfort_box"].clone()).unwrap(),
            )
            .unwrap();
            let mut character = CharacterCoordinator::new(1);
            character.set_gaze_enabled(true);
            character.start(0., &mut core).unwrap();
            let mut desk = Self {
                core,
                boot,
                character,
                azimuth: 0.0,
                elevation: 0.0,
            };
            desk.run(0.0, 5.0, None);
            desk.character.set_curiosity_available(true);
            desk.character.set_conditions(Conditions {
                hour,
                day: 1,
                lamp: LampView::default(),
                ..Default::default()
            });
            let anchor = desk.character.status.active_anchor.clone().unwrap();
            let bearing = crate::control::camera_rays::Bearing::from_direction(
                desk.core
                    .camera_kinematics()
                    .unwrap()
                    .fk(&anchor)
                    .unwrap()
                    .camera
                    .rotate_vector([0.0, 0.0, 1.0]),
            );
            desk.azimuth = bearing.azimuth + 0.2;
            desk.elevation = bearing.elevation;
            desk
        }
        fn observe(&mut self, now: f64, scene: Option<&serde_json::Value>) {
            self.boot
                .store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            let Some(scene) = scene else {
                return;
            };
            let mut value = serde_json::json!({
                "capture_boottime_ns": self.boot.load(Ordering::Relaxed),
                "camera_on": true, "tracking_mode": "idle",
                "settings": {"arrival_notice": true, "mutual_gaze": true, "proactive_speech": true},
                "objects": [], "faces": [], "looking_away": null});
            for (key, item) in scene.as_object().unwrap() {
                value[key] = item.clone();
            }
            if value["faces"] == false {
                value["faces"] = serde_json::json!([]);
            }
            if value["faces"] == true {
                value["faces"] = serde_json::json!([{"track_id": 1, "azimuth": self.azimuth,
                    "elevation": self.elevation, "frontal": false}]);
            }
            self.character
                .observe_surroundings(serde_json::from_value(value).unwrap(), now, &mut self.core)
                .unwrap();
        }
        /// Ticks for some seconds, observing the scene twice a second.
        fn run(&mut self, from: f64, seconds: f64, scene: Option<serde_json::Value>) {
            for step in 1..=(seconds / 0.02).round() as usize {
                let now = from + step as f64 * 0.02;
                self.boot
                    .store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
                self.core.tick(now).unwrap();
                self.character
                    .tick(now, &mut self.core, false, None, false, None, None)
                    .unwrap();
                if step % 25 == 0 {
                    self.observe(now, scene.as_ref());
                }
            }
        }
        fn clip(&self) -> Option<&str> {
            self.character.status.active_clip.as_deref()
        }
    }

    #[test]
    fn lively_orion_waves_back_and_offers_light_in_a_dark_room() {
        let mut desk = Desk::new(19.0);
        let person = serde_json::json!({"faces": true, "room_lux": 300.0});
        desk.run(5.0, 3.0, Some(person.clone()));
        assert_eq!(desk.clip(), None);
        // A palm held up near the face gets a wave back, once per raise.
        let wave = serde_json::json!({"faces": true, "room_lux": 300.0, "raised_hands": 1});
        desk.run(8.0, 1.5, Some(wave.clone()));
        assert_eq!(desk.clip(), Some("wave_back"));
        let mut swayed = false;
        let anchor = desk.character.status.active_anchor.clone().unwrap();
        for step in 1..=400 {
            let now = 9.5 + step as f64 * 0.02;
            desk.run(now - 0.02, 0.02, None);
            swayed |= desk.core.snapshot().joints.iter().any(|joint| {
                joint.name == "base_yaw_joint" && (joint.position - anchor[&joint.name]).abs() > 0.1
            });
        }
        assert!(swayed, "the wave is a visible sway of the arm");
        desk.run(17.5, 4.5, Some(wave));
        assert!(desk.character.surroundings.reaction.is_none());
        assert!(
            desk.character.ready_remark(22.0).is_none(),
            "a wave is silent"
        );
        // The room goes dark with Orion's light off: after a minute, light care.
        let dark = serde_json::json!({"faces": true, "room_lux": 4.0});
        desk.run(22.0, 59.0, Some(dark.clone()));
        assert_eq!(desk.clip(), None);
        desk.run(81.0, 3.0, Some(dark.clone()));
        assert_eq!(desk.clip(), Some("arrival_notice"));
        let remark = desk.character.ready_remark(84.0).unwrap();
        assert_eq!(remark.event, RemarkEvent::LightCare);
        assert_eq!(remark.detail.as_deref(), Some("light off"));
        desk.character.take_remark(remark.id, 84.0).unwrap();
        // Once a visit.
        desk.run(84.0, 30.0, Some(dark));
        assert!(desk.character.ready_remark(114.0).is_none());
    }

    #[test]
    fn a_long_stretch_at_the_desk_gets_a_stretch_and_a_break_nudge() {
        let mut desk = Desk::new(15.0);
        let person = serde_json::json!({"faces": true, "looking_away": true});
        desk.run(5.0, 3.0, Some(person.clone()));
        assert!(desk.character.ready_remark(8.0).is_none());
        // Ninety minutes later the person is still working.
        desk.character.surroundings.lively.desk_since = Some(8.0 - BREAK_SECONDS);
        desk.run(8.0, 1.0, Some(person));
        assert_eq!(desk.clip(), Some("wake_stretch"));
        assert_eq!(
            desk.character.ready_remark(9.0).map(|remark| remark.event),
            Some(RemarkEvent::BreakNudge)
        );
    }

    #[test]
    fn a_late_goodbye_is_a_goodnight_and_orion_rests_unless_someone_returns() {
        let mut desk = Desk::new(22.5);
        let demo = |faces: bool| serde_json::json!({"faces": faces, "settings": {"arrival_notice": true, "demo_mode": true}});
        desk.run(5.0, 25.0, Some(demo(true)));
        desk.character.remarks = Default::default();
        desk.run(30.0, 6.0, Some(demo(false)));
        assert_eq!(desk.clip(), Some("farewell_watch"));
        assert_eq!(
            desk.character.ready_remark(36.0).map(|remark| remark.event),
            Some(RemarkEvent::Goodnight)
        );
        assert!(!desk.character.take_goodnight_rest(100.0));
        // Someone comes back: no early rest.
        desk.run(36.0, 1.0, Some(demo(true)));
        assert!(!desk.character.take_goodnight_rest(200.0));
        // Nobody returns after the next goodnight.
        desk.character.surroundings.lively.goodnight_rest = Some(250.0);
        assert!(!desk.character.take_goodnight_rest(249.0));
        assert!(desk.character.take_goodnight_rest(250.0));
        assert!(!desk.character.take_goodnight_rest(251.0));
    }
}
