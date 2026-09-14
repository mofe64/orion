use serde::Serialize;

use crate::control::core::RuntimeCore;
use crate::control::state::{MovementPhase, RuntimeMode};
use crate::devices::driver::RuntimeDriver;
use crate::error::{OrionRuntimeError as Error, Result};
use crate::expression::speech::SpeechAnalysis;
use crate::motion::library::{
    KeyframeArrival, MotionDefinition, MotionKeyframe, MotionLibrary, MotionSpace,
};
use crate::motion::pose::JointPositions;
use crate::motion::style::MotionStyle;

/// Note:
/// - A clip is a named motion definition, such as "idle_breathe". It describes a movement using joint targets, timing, and style
/// - An anchor is the reference posture around which character motion happens.
///   Suppose Orion is deliberately facing someone to its right. Its speech gestures should happen around that posture, then return to it.
/// - A run ID identifies one particular execution of a movement. The same clip might run many times: e.g.
///         Clip: idle_breathe     Execution: run 41
///         Clip: idle_breathe     Execution: run 58
/// The name lets us know "which movement?" The run ID lets us know "which execution of that movement?"
///

///The following four constants control how long Orion waits before another idle movement becomes eligible
/// The represent waiting delays.
/// The coordinator keeps two independent deadlines. which run in parallel.

/// Shortest randomly selected micro-idle delay.
const MICRO_IDLE_MIN_SECONDS: f64 = 8.0;
/// Longest randomly selected micro-idle delay.
const MICRO_IDLE_MAX_SECONDS: f64 = 20.0;
/// Shortest randomly selected larger-idle delay
const LARGE_IDLE_MIN_SECONDS: f64 = 35.0;
/// Longest randomly selected larger-idle delay.
const LARGE_IDLE_MAX_SECONDS: f64 = 75.0;

/// The following constants are used to tune the rhythm of speech animation

/// For complete audio, this represents the amount of time subtracted from the initial performance time budget.
/// This aims to finish the movement slightly ahead of the audio.
const SPEECH_END_LEAD_SECONDS: f64 = 0.12;
/// The nominal budget reserved for returning to the anchor at the end of a composed speech performance.
const SPEECH_FINAL_SETTLE_SECONDS: f64 = 0.55;
/// Amount by which to scale nominal gesture durations to make them longer before other timing adjustments.
const SPEECH_GESTURE_DURATION_SCALE: f64 = 1.35;
/// Allows the planner to consider upcoming audio peaks within 25 analysis frames.
const SPEECH_PEAK_LOOKAHEAD_FRAMES: usize = 25;
/// Time between planned emphasis selections.
/// an emphasis is a stronger expressive moment selected around an eligible audio peak
/// we use audio energy to determine these peaks
const SPEECH_EMPHASIS_INTERVAL_SECONDS: f64 = 3.5;
/// Difference in gesture indices required between stronger body beat accents.
/// A body beat is a stronger shoulder-and-elbow accent.
/// It has extra conditions:
/// - an eligible emphasis,
/// - sufficiently strong audio energy,
/// - enough spacing from the previous body beat,
/// - and a clip-history check
/// The interval of 3 does not mean always make a body beat every third gesture.
/// If a body beat occurs at gesture index 2, the earliest eligible next index is 5. It may happen much later.
const SPEECH_BODY_BEAT_INTERVAL_DRAWINGS: usize = 3;

const MICRO_IDLES: [&str; 4] = [
    "idle_breathe",
    "idle_head_curiosity",
    "idle_micro_glance",
    "idle_shoulder_adjust",
];
const LARGE_IDLES: [&str; 3] = ["idle_weight_shift", "idle_soft_head_shake", "idle_breathe"];

/// A planned speech drawing is a single drawing to be played during speech.
/// A drawing here means one planned expressive gesture within a speech performance.
/// Its an animation term that refers to poses and movements.
/// This struct acts as a temporary planning object. The composer creates a sequence of these and then
/// converts them into motion keyframes for the robots animation
/// We separate head and body targets, so that we can implement staging and overlapping effects in our animation.
/// the head leads and the body follows.
#[derive(Clone, Debug)]
struct PlannedSpeechDrawing {
    /// The named motion whose authored shape contributes to this gesture.
    clip: String,
    /// Desired head-related offsets: head roll, head pitch, and sometimes base yaw for facing direction.
    head_target: JointPositions,
    /// Desired supporting shoulder and elbow offsets.
    body_target: JointPositions,
    /// The authored movement duration allocated to this gesture, excluding its optional hold.
    duration_seconds: f64,
    /// Whether the body movement is a stronger accent.
    body_beat: bool,
    /// The fraction of movement time allocated to the head-leading stage.
    lead_fraction: f64,
    /// Extra time to retain the gesture’s pose, typically during an audio pause
    hold_seconds: f64,
    /// A snapshot of what speech history should become after this gesture’s body-follow stage and hold have passed.
    memory: SpeechMemory,
}

/// SpeechMemory allows us to plan ahead while keeping track of the part of our movement that has already been performed.
/// this is how we are able to implement animation while streaming audio from out tts model.
/// Suppose Orion has received enough audio to plan gestures A, B, and C.
/// It has finished A when more audio arrives. That additional audio may change what should happen after A.
/// The planner should remember A's history while being free to revise B and C.
/// It acts as temporary animation history that is updated as new audio arrives.
/// So in summary, one SpeechMemory snapshot says something like:
/// "The latest gesture was a nod. These are the recent gestures.
/// The next gesture is number 3. The last strong body accent was gesture 0.
/// The head was tilted this way, the base turned that way, and this was the body shape.
/// We reached this point in the gesture timeline, with the last emphasis at this earlier time.
/// Here is where to resume the random choices."
#[derive(Clone, Debug)]
struct SpeechMemory {
    // Position in the seeded random sequence.
    // Orion uses controlled randomness to vary its gestures.
    // This field saves the random generator’s current state, so planning can resume from that same point later.
    rng: SeededRandom,

    // the latest gesture chosen
    // helps us avoid repeating the same gesture over and over
    clip: Option<String>,

    // a short list of recent gestures
    // allows us to look back and avoid repeating the same gesture too soon
    recent: Vec<String>,

    // the number of the next gesture
    // helps us track our progress in the perfomance/motion
    index: usize,

    // which gesture last had a strong body accent
    // The planner uses this to space those accents apart
    // using our SPEECH_BODY_BEAT_INTERVAL_DRAWINGS.
    body_beat: Option<usize>,

    // the previous sideways head-tilt direction
    // qw uses three direction values:
    // a) -1.0 → tilt in the negative roll direction
    // b)  0.0 → no added sideways tilt
    // c)  1.0 → tilt in the positive roll direction
    tilt: f64,

    // the previous turning direction
    // This is similar to tilt, but concerns turning around the base:
    // -1 → negative yaw direction
    //  0 → no added turn
    //  1 → positive yaw direction
    turn: i8,

    // the body shape to carry forward
    // This remembers the shoulder and elbow offsets associated with the preceding gesture.
    // we need to store this, because the next gesture begins with the head leading and the body retains it previous shape
    // before initating the body follow
    body: JointPositions,
    // how far along the gesture timeline this snapshot is
    seconds: f64,
    // when the latest emphasis was placed
    emphasis_at: Option<f64>,
}

/// CharacterState describes the character activity Orion is presenting.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterState {
    Off,             // Autonomous character behavior is disabled
    Starting, // Character startup is moving toward home and waiting for successful completion.
    HomeIdle, // Idle behavior around a centrally facing anchor.
    PoseIdle, // Idle behavior around another anchor.
    Listening, // Presenting an attentive listening reaction; ordinary autonomous idles are suppressed.
    Thinking, // Presenting thinking behavior, including generated thinking movement when available.
    Speaking, // Speech is active and the coordinator can animate alongside it.
    ForegroundScene, // Explicit foreground work has priority over background character behavior.
    Settling, // Returning toward an anchor or waiting for the relevant return movement to finish.
    ShuttingDown, // Character mode is performing its shutdown return before becoming Off.
}

/// NextIdleCategory identifies which idle schedule is next
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextIdleCategory {
    Micro,
    Large,
}

/// CharacterStatus is the compact view of the coordinator that other parts of the application can read
#[derive(Clone, Debug, Serialize)]
pub struct CharacterStatus {
    /// Whether character mode is enabled.
    pub enabled: bool,
    /// The current character activity.
    pub state: CharacterState,
    /// The reference posture for character movement, if established.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_anchor: Option<JointPositions>,
    /// An optional label for the active character motion or generated performance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_clip: Option<String>,
    /// Which idle deadline is earlier, when a schedule exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_idle_category: Option<NextIdleCategory>,
}

/// Attention tracks a temporary decision to face a speaker.
#[derive(Debug)]
struct Attention {
    /// The anchor to restore after the attention period ends.
    previous: JointPositions,
    /// The currently tracked attention movement, either the outward turn or the return.
    run_id: Option<u64>,
    /// Whether that movement is returning to the previous anchor.
    returning: bool,
    /// A deadline after which a return may begin when conditions permit.
    expires_at: f64,
}

/// CharacterCoordinator manages the character's state and behavior, including idle scheduling and attention tracking.
/// we can think of it as the robot's activity manager
/// With its tick method it asks "What is happening now? Has anything finished? What should happen next?"
/// and it persists info into its fields between ticks so we can track the character's state over time.
#[derive(Debug)]
pub struct CharacterCoordinator {
    /// Character state we expose to other parts of the application.
    status: CharacterStatus,

    /// Supplies repeatable random choices for idle timing and speech gestures.
    /// Saving its state lets speech replanning resume the same choice sequence.
    rng: SeededRandom,

    /// When the next small idle becomes due, in seconds on the runtime clock.
    /// For example, 112.0 means wait until that clock reaches 112 seconds.
    next_micro_at: f64,

    /// When the next larger idle becomes due, on the same runtime clock.
    /// Kept separate so small idles do not keep postponing the larger ones.
    next_large_at: f64,

    /// The last idle clip chosen, so Orion can avoid choosing it twice in a row.
    last_idle: Option<String>,

    /// The tracking number of the idle movement Orion started.
    /// Used to check whether that particular execution has finished.
    active_idle_run_id: Option<u64>,

    /// Whether the running idle came from the micro or large schedule.
    /// When it ends, only that category gets a new deadline.
    active_idle_category: Option<NextIdleCategory>,

    /// The home movement being tracked during character startup or shutdown.
    /// Its result determines when to enter idle or finish turning character mode off.
    starting_run_id: Option<u64>,

    /// A directly requested foreground action still needs its final posture captured.
    /// Once movement ends, that posture becomes the anchor for later character motion.
    foreground_pending: bool,

    /// The foreground scene whose result Orion is waiting for.
    /// Only successful completion of this scene replaces the character's anchor.
    foreground_scene_run_id: Option<u64>,

    /// The current thinking movement, so Orion can wait for it, repeat it when
    /// finished, or hand movement over when speech or another activity takes priority.
    thinking_run: Option<u64>,

    /// The movement run used for speech gestures and their final return to the anchor.
    /// This tracks motion, not audio playback; streaming extensions keep this run ID.
    speech_motion_run_id: Option<u64>,

    /// Whether Orion has already attempted to start animation for this utterance.
    /// Prevents every tick from starting it again, even if the first attempt failed.
    speech_motion_started: bool,

    /// The latest speech clip remembered by the planner.
    /// Excluded from the next choice to avoid immediately repeating the same shape.
    last_speech_clip: Option<String>,

    /// The received audio length used for the latest speech-planning attempt,
    /// in seconds from the utterance's start. More audio can require a longer plan.
    speech_planned_until: f64,

    /// Whether the latest speech plan expected more audio to arrive.
    /// The stream ending requires a final return even if no extra audio arrives.
    speech_plan_streaming: bool,

    /// The next gesture number in the speech history.
    /// Continuing this count across audio chunks keeps gesture spacing consistent.
    speech_gesture_index: usize,

    /// The gesture number of the latest strong shoulder-and-elbow accent, if any.
    /// Used to leave enough ordinary gestures between stronger body movements.
    speech_last_body_beat: Option<usize>,

    /// The previous head-roll direction: -1.0, 0.0, or 1.0, rather than an angle.
    /// Lets the next gesture keep that tilt, ease toward neutral, or change sides.
    speech_last_tilt: f64,

    /// The previous base-yaw direction: -1, 0, or 1, rather than an angle.
    /// Helps choose a different direction for the next speech gesture.
    speech_last_turn: i8,

    /// The preceding gesture's shoulder and elbow offsets from the anchor, in radians.
    /// The body retains this shape while the next gesture's head movement leads.
    speech_previous_body: JointPositions,

    /// Recent speech clip names, used to make familiar shapes less likely to recur.
    /// This adds variety beyond simply excluding the immediately previous clip.
    speech_recent_clips: Vec<String>,

    /// How much gesture time the retained speech history represents, in seconds.
    /// Carries timing across replans; this is not the runtime clock or live audio position.
    speech_seconds: f64,

    /// Where the latest emphasis falls on the gesture timeline, in seconds.
    /// Used to keep stronger gestures spaced apart when speech is extended.
    speech_emphasis_at: Option<f64>,

    /// Bookmarks pairing a body-follow keyframe index with its future speech history.
    /// Adopt that history only after movement passes the keyframe and its hold,
    /// so replanning does not treat unfinished gestures as already performed.
    speech_checkpoints: Vec<(usize, SpeechMemory)>,

    /// A temporary decision to face a speaker, including the previous anchor
    /// to return to and the deadline for returning when movement is available.
    attention: Option<Attention>,
}

impl CharacterCoordinator {
    /// Creates a new CharacterCoordinator with the given random seed.
    /// Fields are initialized to their default values, ready for the first tick.
    pub fn new(seed: u64) -> Self {
        Self {
            status: CharacterStatus {
                enabled: false,
                state: CharacterState::Off,
                active_anchor: None,
                active_clip: None,
                next_idle_category: None,
            },
            rng: SeededRandom::new(seed),
            next_micro_at: f64::INFINITY,
            next_large_at: f64::INFINITY,
            last_idle: None,
            active_idle_run_id: None,
            active_idle_category: None,
            starting_run_id: None,
            foreground_pending: false,
            foreground_scene_run_id: None,
            thinking_run: None,
            speech_motion_run_id: None,
            speech_motion_started: false,
            last_speech_clip: None,
            speech_planned_until: 0.0,
            speech_plan_streaming: false,
            speech_gesture_index: 0,
            speech_last_body_beat: None,
            speech_last_tilt: 0.0,
            speech_last_turn: 0,
            speech_previous_body: JointPositions::new(),
            speech_recent_clips: Vec::new(),
            speech_seconds: 0.0,
            speech_emphasis_at: None,
            speech_checkpoints: Vec::new(),
            attention: None,
        }
    }

    /// Resets the character's attention state, clearing any active attention movement.
    pub fn clear_attention(&mut self) {
        self.attention = None;
    }

    /// Starts an authored turn toward a supplied left/right speaker direction.
    ///
    /// Rejects invalid or low-confidence requests, unavailable character states,
    /// and turns that would require too much base rotation.
    /// May interrupt an idle or thinking movement.
    ///
    /// Saves the existing anchor, starts the attention motion, and records its
    /// run ID and initial expiry deadline. Returns without waiting for arrival.
    ///
    /// After successful arrival, tick() captures the facing posture as a temporary
    /// anchor. While attention remains active, tick() suppresses ordinary idles
    /// and starts the return to the saved anchor once attention has expired,
    /// speech is inactive, and the runtime is holding.
    ///
    /// Interruption or foreground work can end this attention sequence early.
    pub fn attend<D: RuntimeDriver>(
        &mut self,
        side: &str,
        confidence: f64,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<CharacterStatus> {
        // Reject the request if the direction is unsupported, the confidence isn’t a finite number, or the confidence is outside the accepted range.
        if !matches!(side, "left" | "right")
            || !confidence.is_finite()
            || !(0.75..=1.0).contains(&confidence)
        {
            return Err(Error::InvalidArgument(
                "Attention requires left/right and confidence in [0.75, 1].".into(),
            ));
        }
        // we check whether attention is allowed to take control of movement.
        // we do not allow attention to take control when
        // - Character mode is disabled.
        // - A startup or shutdown movement is being tracked.
        // - Foreground work is pending or a foreground scene is being tracked.
        // - Speech movement is being tracked.
        // - The character isn't idle, listening, or thinking.
        // - The runtime is moving, but the coordinator isn't tracking that movement as an idle or thinking movement.
        if !self.status.enabled
            || self.starting_run_id.is_some()
            || self.foreground_pending
            || self.foreground_scene_run_id.is_some()
            || self.speech_motion_run_id.is_some()
            || !matches!(
                self.status.state,
                CharacterState::HomeIdle
                    | CharacterState::PoseIdle
                    | CharacterState::Listening
                    | CharacterState::Thinking
            )
            // this condition essentially means the something is moving,
            // and it isn’t one of the two lower-priority activities (idle, thinking)
            // that attention is allowed to interrupt.
            || (core.mode() == RuntimeMode::Moving
                && self.active_idle_run_id.is_none()
                && self.thinking_run.is_none())
        {
            return Err(Error::InvalidState(
                "Attention requires an available powered character.".into(),
            ));
        }
        // check if attention is already active, this means the character is already facing the user
        // and we don't need to do anything.
        if self.attention.is_some() {
            return Ok(self.status.clone()); // One facing decision per conversation.
        }

        // we save the anchor so we can restore it after attention is done.
        let previous = self
            .status
            .active_anchor
            .clone()
            .ok_or_else(|| Error::InvalidState("Attention needs an anchor.".into()))?;
        // calculate the target yaw based on the side we're facing.
        let target_yaw = if side == "left" { -0.35 } else { 0.35 };

        // using our runtime, we read the latest joint feedback
        // find the base_yaw_joint and read its position.
        // we do this to check if  applying our target to the current position
        // would make the character turn too far. (our limit here is 0.65 radians approx 37 degrees)
        // if it would, we return an error
        if (core
            .snapshot()
            .joints
            .iter()
            .find(|joint| joint.name == "base_yaw_joint")
            .ok_or_else(|| Error::Runtime("Missing base feedback".into()))?
            .position
            - target_yaw)
            .abs()
            > 0.65
        {
            return Err(Error::InvalidState(
                "Attention would require a broad turn from this pose.".into(),
            ));
        }
        // preempt any running idle or thinking run
        self.preempt_idle_or_thinking(now, core)?;
        // we ask the runtime to start the appropriate motion
        // checked reads the command response and returns an error our runtime reported failure
        let response = checked(core.handle_command(&format!("play attention_{side}"), now))?;
        // extract the run id from our response
        let run_id = response
            .get("run_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| Error::Runtime("Attention has no run ID.".into()))?;
        // update our attention state
        // we track the previous anchor, the run id, and the deadline for 2 minutes
        // returning is false, as we are starting a new attention run
        self.attention = Some(Attention {
            previous,
            run_id: Some(run_id),
            returning: false,
            expires_at: now + 120.0,
        });
        // update the character state to reflect
        // we are now listening to the user
        // we are currently playing the attention clip
        // we reset both idle timers
        self.status.state = CharacterState::Listening;
        self.status.active_clip = Some(format!("attention_{side}"));
        self.reset_timers(now);
        // return copy of the updated status
        Ok(self.status.clone())
    }

    /// Returns a reference to the character's current status struct.
    pub fn status(&self) -> &CharacterStatus {
        &self.status
    }

    /// Select the lowest-priority character light for the current held state.
    pub fn background_lighting_effect<D: RuntimeDriver>(
        &self,
        core: &RuntimeCore<D>,
    ) -> Option<String> {
        // if character mode is disabled, return early
        if !self.status.enabled {
            return None;
        }
        // determine the background lighting effect based on the current state
        match self.status.state {
            CharacterState::Listening => return Some("attentive_focus".into()),
            CharacterState::Thinking => return Some("thinking_drift".into()),
            CharacterState::Starting | CharacterState::Settling => {
                return Some("settle_glow".into());
            }
            CharacterState::Off
            | CharacterState::Speaking
            | CharacterState::ForegroundScene
            | CharacterState::ShuttingDown => return None,
            CharacterState::HomeIdle | CharacterState::PoseIdle => {}
        }
        // if we can't determine the lighting effect based on the state,
        // we delegate to the closest_pose_default_lighting which uses the current anchor
        // to determine the lighting effect
        // if no anchor is available, we fall back to the default lighting effect warm_idle_breathe
        self.status
            .active_anchor
            .as_ref()
            .and_then(|anchor| closest_pose_default_lighting(core, anchor))
            .or_else(|| Some("warm_idle_breathe".into()))
    }

    /// Starts the character mode, enabling it and configuring the runtime if necessary.
    /// Returns an error if the character mode is already enabled or if the runtime is in an invalid mode.
    /// It also rejects another start() while the first startup is still in progress,
    /// because the method sets enabled to true as soon as startup is successfully initiated.
    pub fn start<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<CharacterStatus> {
        if self.status.enabled {
            // Character mode is already enabled, return an error.
            return Err(Error::InvalidState(
                "Character mode is already enabled.".into(),
            ));
        }
        // we use the runtime mode which describes whether the motion system is observing, configured, holding, or moving.
        // if we are in observe mode, we init the configure comand, which will ask the driver to apply the servo settings
        // after successful configuration, the runtime enters configured mode
        if core.mode() == RuntimeMode::Observe {
            checked(core.handle_command("configure", now))?;
        }
        // if we are in configured mode, we send the enable command to the runtime
        // this enables holding torque which alows the motors to actively maintain their joint positions
        // after enabling, the runtime enters holding mode
        if core.mode() == RuntimeMode::Configured {
            checked(core.handle_command("enable", now))?;
        }
        // confirm that we are in holding mode
        // if at this point, we are not in holding mode, the enable command failed or there is another movement in progress
        // so we return an error indicating that character mode requires configured holding torque
        if core.mode() != RuntimeMode::Holding {
            return Err(Error::InvalidState(
                "Character mode requires configured holding torque.".into(),
            ));
        }

        // we move to the home position using the goto home command
        // the runtime build the trajectory to the named home pose, using the requested travel duration of 1.6 seconds
        // and the runtime also tracks the execution of this movement
        // now tells the runtime when this movement began on its elapsed-time clock.
        // we use a goto command here with a provided duration, we aren't using the return_home motion (which is defined in our yaml)
        let response = checked(core.handle_command("goto home 1.600000", now))?;
        // record the movemnet so that our character state can track it on tick
        self.starting_run_id = response.get("run_id").and_then(serde_json::Value::as_u64);
        self.status.enabled = true;
        self.status.state = CharacterState::Starting;
        self.status.active_clip = Some("return_home".into());

        // in our tick method, the coordinator will check the execution of this movement using the starting_run_id
        // If startup completed successfully, it captures the measured posture as the anchor, enters HomeIdle, clears the clip label, and resets the idle timers again.
        // If startup was cancelled or timed out, it switches character mode off.

        // reset idle timers
        self.reset_timers(now);
        // return copied status
        Ok(self.status.clone())
    }

    /// Stops the character's current movement and resets the character state. and begin the shutdown movement.
    pub fn stop<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<CharacterStatus> {
        // If the character is not enabled, return an error.
        if !self.status.enabled {
            return Err(Error::InvalidState("Character mode is not enabled.".into()));
        }
        // clear the temporary attention state
        self.clear_attention();
        // stopes and clear any tracked idle or thinking activity
        self.preempt_idle_or_thinking(now, core)?;

        // if runtime is moving, stop it
        // the stop comamnd cancels any active movement plan and changes it move to holding
        if core.mode() == RuntimeMode::Moving {
            checked(core.handle_command("stop", now))?;
        }

        // now if we are in holding, we start the shutdown return home sequence
        // this time we use the return home motion defined in our yaml instead of a goto command
        if core.mode() == RuntimeMode::Holding {
            let response = checked(core.handle_command("play return_home", now))?;
            // update our state to reflect that shutdown is in progress
            // note starting_run_id tracks the home movement for both startup and shutdown.
            // tick() uses the character state to distinguish the two cases.
            // also, we don't set character mode enabled to false here, this is because we want future ticks to
            // track the shutdown movement, what we do here is leave enbled true and set state to ShuttingDown.
            // future ticks will call finish_stop() when the shutdown movement is complete.
            self.starting_run_id = response.get("run_id").and_then(serde_json::Value::as_u64);
            self.status.state = CharacterState::ShuttingDown;
            self.status.active_clip = Some("return_home".into());
        } else {
            // if we're not in holding mode, we can immediately call finish_stop()
            // finish_stop() clears the character’s status and movement tracking:
            self.finish_stop();
        }
        // return the updated status
        Ok(self.status.clone())
    }

    /// Places our robot in a resting state using the existing rest trajectory.
    pub fn rest<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<serde_json::Value> {
        // clear attention and preempt any idle or thinking movement
        self.clear_attention();
        self.preempt_idle_or_thinking(now, core)?;
        // stop moving if we're in moving mode
        if core.mode() == RuntimeMode::Moving {
            checked(core.handle_command("stop", now))?;
        }
        // clears the character's status and movement tracking:
        self.finish_stop();

        // if in observe mode, we run the configure comand, which will ask the driver to apply the servo settings
        // after successful configuration, the runtime enters configured mode
        if core.mode() == RuntimeMode::Observe {
            checked(core.handle_command("configure", now))?;
        }

        // if in configured mode, we run the enable command, this enables holding torque which alows the motors to actively maintain their joint positions
        // after enabling, the runtime enters holding mode
        if core.mode() == RuntimeMode::Configured {
            checked(core.handle_command("enable", now))?;
        }
        // run the goto rest command to move the character to our rest position
        checked(core.handle_command("goto rest 3.0", now))
    }

    /// Requests a neutral, listening, or thinking character reaction.
    /// Character mode must be enabled, and startup, shutdown, and foreground
    /// work keep priority over reaction changes.
    ///
    /// Stops tracked idle or thinking movement and updates the idle and attention
    /// timers. Speaking and settling keep their state until tick() finishes
    /// handling them. Otherwise, the requested state guides later movement and
    /// background lighting.
    pub fn set_reaction<D: RuntimeDriver>(
        &mut self,
        reaction: &str,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<CharacterStatus> {
        // A reaction needs an enabled character session.
        if !self.status.enabled {
            return Err(Error::InvalidState(
                "Enable character mode before setting character state.".into(),
            ));
        }
        // If we have startup, shutdown, and foreground work unfinished we are not allowed to change reactions.
        // starting_run_id tracks the home movement for both startup and shutdown.
        if self.starting_run_id.is_some()
            || self.foreground_pending
            || self.foreground_scene_run_id.is_some()
        {
            return Err(Error::InvalidState(
                "Character transition has priority over a reaction.".into(),
            ));
        }
        // Interrupt any idle or thinking movement and reset timers for idle movement.
        self.preempt_idle_or_thinking(now, core)?;
        self.reset_timers(now);

        // If attention is active, we use the reaction to determine the attention wait time.
        // neutral reaction sets a shorter wait, while other reactions set a longer wait.
        if let Some(attention) = self.attention.as_mut() {
            attention.expires_at = now + if reaction == "neutral" { 15.0 } else { 120.0 };
        }

        // if we are speaking or settling, and the reaction is neutral, listening, or thinking,
        // we return early to avoid preempting the current movement.
        if matches!(
            self.status.state,
            CharacterState::Speaking | CharacterState::Settling
        ) && matches!(reaction, "neutral" | "listening" | "thinking")
        {
            return Ok(self.status.clone());
        }

        // set the character state based on the reaction
        // return error if the reaction is not valid
        match reaction {
            "neutral" => self.status.state = self.idle_state(),
            "listening" => self.status.state = CharacterState::Listening,
            "thinking" => self.status.state = CharacterState::Thinking,
            _ => {
                return Err(Error::InvalidArgument(
                    "Character state must be neutral, listening, or thinking.".into(),
                ));
            }
        }
        // Return a copy of the updated status
        Ok(self.status.clone())
    }

    /// Stops tracked idle or thinking movement so another activity can take over.
    /// Clears the associated tracking information, including runs that have
    /// already ended
    pub fn preempt_idle_or_thinking<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        // if we have an active idle or thinking run, and the runtime is still moving,
        // send a stop command to cancel the movement
        if (self.active_idle_run_id.is_some() || self.thinking_run.is_some())
            && core.mode() == RuntimeMode::Moving
        {
            checked(core.handle_command("stop", now))?;
        }
        // forget the thinking run, even if it had already finished
        self.thinking_run = None;
        // take() clears the idle run ID and tells us whether one was present.
        // If it was, also clear the clip label that described that idle.
        if self.active_idle_run_id.take().is_some() {
            self.status.active_clip = None;
        }
        // Forget which idle schedule this run belonged to.
        self.active_idle_category = None;
        Ok(())
    }

    /// Records that the caller has started a direct foreground movement.
    /// Clears attention and tracking for a home movement, then gives the
    /// foreground action priority while character mode is enabled.
    pub fn note_foreground_started(&mut self, now: f64) {
        // clear attention and tracking for a home movement
        self.clear_attention();
        self.starting_run_id = None;
        // if character mode is not enabled, do nothing
        if !self.status.enabled {
            return;
        }
        // update state to reflect foreground scene
        // set foreground pending to true so tick() will capture the resulting posture
        self.foreground_pending = true;
        self.status.state = CharacterState::ForegroundScene;
        self.status.active_clip = None;
        self.reset_timers(now);
    }

    /// Records that audio playback has started for a new utterance.
    /// Leaves disabled character mode and tracked startup or shutdown unchanged.
    pub fn note_speech_started(&mut self, now: f64) {
        // Speech does not enable character mode or take over its home transition.
        // so we return early if character mode is not enabled or we are still starting up
        if !self.status.enabled || self.starting_run_id.is_some() {
            return;
        }

        // set state to speaking and clear the old clip label
        // then reset timers and set speech fields to their default values
        self.status.state = CharacterState::Speaking;
        self.status.active_clip = None;
        self.speech_motion_started = false;
        self.reset_timers(now);
        self.speech_planned_until = 0.0;
        self.speech_plan_streaming = false;
        // Start gesture counting, direction choices, and body-follow history afresh.
        self.speech_gesture_index = 0;
        self.speech_last_body_beat = None;
        self.speech_last_tilt = 0.0;
        self.speech_last_turn = 0;
        self.speech_previous_body.clear();
        self.speech_recent_clips.clear();
        // Restart the speech timeline and discard checkpoints from the old plan.
        self.speech_seconds = 0.0;
        self.speech_emphasis_at = None;
        self.speech_checkpoints.clear();
    }

    /// Records a foreground scene that the caller has already started.
    /// The supplied run ID belongs to the whole scene, which may coordinate
    /// movement, lighting, and audio.
    pub fn note_foreground_scene_started(&mut self, now: f64, run_id: u64) {
        // Forget attention and tracking for a home movement as the scene takes over.
        // The caller handles any movement cancellation before notifying us.
        self.clear_attention();
        self.starting_run_id = None;
        // if character mode is not enabled, return early
        if !self.status.enabled {
            return;
        }
        // Save this scene's ID so tick() can match its completion result.
        self.foreground_scene_run_id = Some(run_id);
        // Report foreground activity, clear the old clip label, and reset idle delays.
        self.status.state = CharacterState::ForegroundScene;
        self.status.active_clip = None;
        self.reset_timers(now);
    }

    /// Ticks the character state machine, updating its state based on the current time and input.
    /// earlier method like start, attend, note_* start an activity or record that something has happend,
    /// tick then repetedly checks progress and decides what to do next.
    /// the runtime calls it every 20ms. each call advances the state machine by one step.
    /// Arguments
    /// now - Current time on the runtime's elapsed-time clock.
    /// core - Access to motion commands, joint feedback, and movement progress.
    /// scene_active - Whether a foreground scene is currently running.
    /// last_scene_result - An optional pair containing a scene's run ID and whether it completed successfully.
    /// speech_active - Whether speech audio is currently playing.
    /// speech_analysis - Optional information about the audio's energy, peaks, pauses, and duration.
    /// speech_frame - The current position in the audio analysis, counted in 20 ms frames.
    pub fn tick<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
        scene_active: bool,
        last_scene_result: Option<(u64, bool)>,
        speech_active: bool,
        speech_analysis: Option<&SpeechAnalysis>,
        speech_frame: Option<usize>,
    ) -> Result<()> {
        // no character processing if character mode is not enabled
        if !self.status.enabled {
            return Ok(());
        }

        // handle tracked startup or shutdown movement
        // if we have a tracked starting run id value
        if let Some(run_id) = self.starting_run_id {
            // evaluate the run using its id and the runtime, to check if we are in a terminal phase (completed, cancelled, timed out)
            if let Some(phase) = terminal_phase(core, run_id) {
                // if we are in a terminal phase,
                // clear out our tracked run id
                self.starting_run_id = None;
                //if we are shutting down
                if self.status.state == CharacterState::ShuttingDown {
                    // initiate the finish stop procedure and return
                    self.finish_stop();
                    return Ok(());
                }
                // if we get here this means we are not in shutdown mode, we are starting mode
                // we check to see if the phase is not completed, this means this is a terminal non completed like cancelled or timed out
                // in that case we initiate the finish stop procedure and return
                if phase != MovementPhase::Completed {
                    self.finish_stop();
                    return Ok(());
                }

                // if we get here this means we are in starting mode and the phase is completed
                // we capture the anchor, set the state to home idle, and reset the timers
                self.capture_anchor(core);
                self.status.state = CharacterState::HomeIdle;
                self.status.active_clip = None;
                self.reset_timers(now);
            }
            return Ok(());
        }

        // if a scene is active, we set the state to foreground scene and return
        // A scene is running. Report that activity and leave character movement decisions alone for this tick.
        if scene_active {
            self.status.state = CharacterState::ForegroundScene;
            return Ok(());
        }

        // if we get here, it means a scene is not active
        // so we check if we are still waiting for a scene to complete
        if let Some(run_id) = self.foreground_scene_run_id {
            // check to see if the last scene has a result, if not we return and wait for the next tick
            let Some((terminal_run_id, completed)) = last_scene_result else {
                return Ok(());
            };
            // if the last scene result does have the same id as the foreground scene we are waiting on,
            // we return and wait for the next tick
            // we match the id to prevent a old scene's result from being mistaken as the current scene's result
            if terminal_run_id != run_id {
                return Ok(());
            }

            // at this point, we have a terminal result for our foreground scene
            // we check to see if it completed
            if completed {
                // if it completed, we capture the anchor so that we establish a new reference pose for subsequent character behaviors
                self.capture_anchor(core);
            }
            // clear the foreground scene run id, state, and reset the timers
            self.foreground_scene_run_id = None;
            self.status.state = self.idle_state();
            self.status.active_clip = None;
            self.reset_timers(now);
        }

        // we check to see if the active anchor is a shutdown-only pose
        // if it is, we finish the stop and return
        // we don't allow character movements while in shutdown pose
        if self
            .status
            .active_anchor
            .as_ref()
            .is_some_and(|anchor| closest_pose_is_shutdown_only(core, anchor))
        {
            self.finish_stop();
            return Ok(());
        }

        // attention sequence mgmt
        // we check to see if we have an attention state running
        // take() removes the attention record from self.attention, leaving None, and gives this block the record to work with.
        // The method then decides whether to put it back.
        if let Some(mut attention) = self.attention.take() {
            if let Some(run_id) = attention.run_id {
                // check to see if our attention run has reached a terminal phase
                if let Some(phase) = terminal_phase(core, run_id) {
                    // if it has, we clear the run_id and active_clip, and handle the phase
                    attention.run_id = None;
                    self.status.active_clip = None;
                    // if the attention run completed,
                    if phase == MovementPhase::Completed {
                        // if our robot is currently returning from an attention run, which means we are no longer thinking or listening to the speaker,
                        // we restore the pose we were in before the attention run started, and set state to idle, and reset idle timers
                        if attention.returning {
                            self.status.active_anchor = Some(attention.previous.clone());
                            self.status.state = self.idle_state();
                            self.reset_timers(now);
                        } else {
                            // if we are not yet returning, we are still turned towards speaker either listening or thinking,
                            // so we capture the current anchor and keep the attention run active
                            self.capture_anchor(core);
                            self.attention = Some(attention);
                        }
                    }
                }
                // if attention is still active, but speech starts, then speech takes precedence, so we stop the attention run, clear the active clip
                // we don't return here, since the tick method still has to handle the speech case, this is just to clean up the attention state
                else if speech_active {
                    checked(core.handle_command("stop", now))?;
                    self.status.active_clip = None;
                } else {
                    // else speech is not active, so we restore the attention state and return
                    self.attention = Some(attention);
                    return Ok(());
                }
            }
            // this case handles if there is no speech interruption, we are still holding our attention, but the attention hold period has expired
            else if !speech_active
                && now >= attention.expires_at
                && core.mode() == RuntimeMode::Holding
            {
                // in that case we should return to the previous anchor
                // we use our runtime to play the attention return motion
                attention.run_id = Some(core.play_generated_anchored_relative(
                    attention_return_motion(),
                    attention.previous.clone(),
                    now,
                )?);
                // we mark the attention as returning so we can track it
                // update the state to track the attention return motion and return
                attention.returning = true;
                self.status.active_clip = Some("attention_return".into());
                self.status.state = CharacterState::Settling;
                self.attention = Some(attention);
                return Ok(());
            } else {
                self.attention = Some(attention);
            }
        }

        // if speech is currently playing
        if speech_active {
            // if we aren't already in speaking state, we capture our current anchor if we don't have one,
            // update the state to speaking, and clear our speech motion flag, this flag prevents speech animation start attempt
            if self.status.state != CharacterState::Speaking {
                if self.status.active_anchor.is_none() {
                    self.capture_anchor(core);
                }
                self.status.state = CharacterState::Speaking;
                self.speech_motion_started = false;
            }
            // we delegate work to tick_speaking which uses audio analysis to start, extend,
            // or finish planning the speech performance. It also advances the remembered gesture history as execution passes checkpoints.
            self.tick_speaking(now, core, speech_analysis, speech_frame);
            return Ok(());
        }

        // if we get here, this means that audio has stopped, but the character is still marked as speaking
        if self.status.state == CharacterState::Speaking {
            // check to see if we have a speech motion run id
            if let Some(run_id) = self.speech_motion_run_id {
                // check to see if the terminal phase has been reached for the current speech motion
                if terminal_phase(core, run_id).is_none() {
                    // check to see if we are already returning, this is evidenced by active_clip being "speak_settle"
                    // or the runtime has reached motion settling for the current speech run id
                    // "speak_settle" identifies a motion that returns toward the anchor.
                    // MovementPhase::Settling means the runtime has finished the planned travel and is checking whether the physical robot has sufficiently settled.
                    if self.status.active_clip.as_deref() == Some("speak_settle")
                        || core.snapshot().motion.as_ref().is_some_and(|m| {
                            m.run_id == run_id && m.state == MovementPhase::Settling
                        })
                    {
                        // if we are already returning, set the state to Settling and return early
                        self.status.state = CharacterState::Settling;
                        return Ok(());
                    }
                    // if we get here, audio has stopped, character is still marked as Speaking, but the motion is not yet settling
                    // so we try to replace the remaning speech gestures with a return to anchor
                    // The runtime starts the replacement from the position and velocity commanded by the existing trajectory at this moment
                    // is says Continue smoothly from the current commanded movement, but make the remaining path lead back to the anchor.
                    if let Some(anchor) = self.status.active_anchor.clone()
                        && core
                            .extend_character_performance(
                                run_id,
                                speech_settle_motion(),
                                anchor,
                                now,
                            )
                            .is_ok()
                    {
                        // if our replacement succeeds, mark the clip as "speak_settle" and the state as Settling then return early
                        self.status.active_clip = Some("speak_settle".into());
                        self.status.state = CharacterState::Settling;
                        eprintln!(
                            "{}",
                            serde_json::json!({"event": "speech.motion_settle", "motion_run_id": run_id, "handover": "commanded"})
                        );
                        return Ok(());
                    }
                    // if we get here, that means our replacement was unsuccessful, so we go to our fallback sequenc
                    // Attempt to stop any movement still running.
                    // Clear the old speech run ID.
                    // If the runtime is holding and an anchor exists, start a new return movement.
                    // Store the new run ID and report Settling.
                    if core.mode() == RuntimeMode::Moving {
                        let _ = checked(core.handle_command("stop", now));
                    }
                    self.speech_motion_run_id = None;
                    if core.mode() == RuntimeMode::Holding
                        && let Some(anchor) = self.status.active_anchor.clone()
                        && let Ok(settle_run_id) = core.play_generated_anchored_relative(
                            speech_settle_motion(),
                            anchor,
                            now,
                        )
                    {
                        self.speech_motion_run_id = Some(settle_run_id);
                        self.status.active_clip = Some("speak_settle".into());
                        self.status.state = CharacterState::Settling;
                        eprintln!(
                            "{}",
                            serde_json::json!({"event": "speech.motion_settle", "motion_run_id": settle_run_id, "handover": "measured_fallback"})
                        );
                        return Ok(());
                    }
                }

                self.speech_motion_run_id = None;
            }

            // if we get here, it means there is no speech moevement left to track, or our recovery attempts cannot establish a return
            // so we clear speech activity and move back to idle state
            self.status.active_clip = None;
            self.status.state = self.idle_state();
            self.reset_timers(now);
        }

        // clean up for tracked speech motion
        if self.status.state == CharacterState::Settling {
            // check to see if we have a speech motion run id, if we do

            if let Some(run_id) = self.speech_motion_run_id {
                // check to see if we have a terminal result
                if terminal_phase(core, run_id).is_none() {
                    // if we don't we return and wait for next tick
                    return Ok(());
                }
                // if we do have a terminal result, clear the run id, this means that speech motion is done
                self.speech_motion_run_id = None;
            }
            // clear any active clip and move to idle state
            // reset idle timers
            self.status.active_clip = None;
            self.status.state = self.idle_state();
            self.reset_timers(now);
        }

        // if we have a tracked thinking run, we check to see if we have a terminal result
        // if we don't, we return and wait for next tick
        // otherwise we clear the old run id and if we're in holding mode and an anchor exists, we start a new thinking motion around that anchor
        if self.status.state == CharacterState::Thinking {
            if self
                .thinking_run
                .is_some_and(|run| terminal_phase(core, run).is_none())
            {
                return Ok(());
            }
            self.thinking_run = None;
            if core.mode() == RuntimeMode::Holding {
                if let Some(anchor) = self.status.active_anchor.clone() {
                    self.thinking_run = core
                        .play_generated_anchored_relative(thinking_motion(), anchor, now)
                        .ok();
                }
            }
            return Ok(());
        }

        // check any tracked idle run for terminal phase
        // If there is no terminal result, it returns and waits.
        // If the idle has ended, it:
        // 1. Clears the tracked idle run ID.
        // 2. Retrieves and clears its category: micro or large.
        // 3. Clears the active clip label.
        // 4. Restores the appropriate idle state.
        // 5. Reschedules only the category that just finished.
        if let Some(run_id) = self.active_idle_run_id {
            if let Some(phase) = terminal_phase(core, run_id) {
                self.active_idle_run_id = None;
                let category = self.active_idle_category.take().ok_or_else(|| {
                    Error::Runtime("Autonomous idle lost its scheduling category.".into())
                })?;
                self.status.active_clip = None;
                self.status.state = self.idle_state();
                self.reschedule_idle(category, now);

                // check that the phase is terminal
                debug_assert!(phase.is_terminal());
            }
            return Ok(());
        }

        // handle the ending of a directly requested foreground movement
        // we check to see that we have a pending foreground movement, and the runtime no longer reports an active movement
        // so we can safely capture the anchor and reset the foreground pending flag
        if self.foreground_pending && core.snapshot().motion.is_none() {
            self.capture_anchor(core);
            self.foreground_pending = false;
            // set to settling and then idle
            self.status.state = CharacterState::Settling;
            self.status.state = self.idle_state();
            self.reset_timers(now);
        }
        // at this stage, we start planning for new idle movement

        // before moving to idle, character state must be home or pose idle, and the runtime must be holding
        // if not we return
        if !matches!(
            self.status.state,
            CharacterState::HomeIdle | CharacterState::PoseIdle
        ) || core.mode() != RuntimeMode::Holding
        {
            return Ok(());
        }

        // if we are under attention, we return early, no idle while holding attention
        // This lets Orion retain its deliberate speaker-facing direction without an ordinary idle delaying the attention return.
        if self.attention.is_some() {
            return Ok(());
        }
        // if we get here, that means we are allowed to idle
        // chose the idle category based on which deadline is closer
        let category = if self.next_micro_at <= self.next_large_at {
            NextIdleCategory::Micro
        } else {
            NextIdleCategory::Large
        };
        // set the idle category in the status
        self.status.next_idle_category = Some(category);

        // calculate the due time for our idle animation based on the idle category
        let due = match category {
            NextIdleCategory::Micro => self.next_micro_at,
            NextIdleCategory::Large => self.next_large_at,
        };

        // if we haven't reached the due time, return early
        if now < due {
            return Ok(());
        }
        // choose an idle clip based on the idle category
        let clip = self.choose_idle(category, core);
        // we check for our anchor to play the idle clip relative to
        let anchor =
            self.status.active_anchor.clone().ok_or_else(|| {
                Error::InvalidState("Character idle has no immutable anchor.".into())
            })?;
        // play the idle clip relative to our anchor
        let run_id = core.play_anchored_relative(&clip, anchor, now)?;
        // update our idle run state
        self.active_idle_run_id = Some(run_id);
        self.active_idle_category = Some(category);
        self.status.active_clip = Some(clip.clone());
        self.last_idle = Some(clip);
        // return
        Ok(())
    }

    /// Choose an idle clip based on the idle category and the character's anchor.
    fn choose_idle<D: RuntimeDriver>(
        &mut self,
        category: NextIdleCategory,
        core: &RuntimeCore<D>,
    ) -> String {
        // get the closest pose profile based on the character's anchor
        let profile = self
            .status
            .active_anchor
            .as_ref()
            .and_then(|anchor| closest_pose_profile(core, anchor));

        // get the list of candidates based on the profile and category
        let mut candidates: Vec<&str> = match (profile.as_deref(), category) {
            (Some("directional"), NextIdleCategory::Micro) => vec![
                "idle_breathe",
                "idle_head_curiosity",
                "idle_shoulder_adjust",
                "idle_directional_hold",
            ],
            (Some("directional"), NextIdleCategory::Large) => {
                vec!["idle_breathe", "idle_weight_shift", "idle_directional_hold"]
            }
            (_, NextIdleCategory::Micro) => MICRO_IDLES.to_vec(),
            (_, NextIdleCategory::Large) => LARGE_IDLES.to_vec(),
        };
        // if our profile is attentive, add the attentive hold clip
        if profile.as_deref() == Some("attentive") {
            candidates.push("idle_attentive_hold");
        }
        // remove any clips that match our last idle clip
        candidates.retain(|clip| self.last_idle.as_deref() != Some(*clip));
        // pick a random clip from the remaining candidates
        candidates[self.rng.index(candidates.len())].to_owned()
    }

    /// Take a snapshot of the speech-planning history.
    /// creates and returns a separate SpeechMemory value containing the coordinator’s current speech-planning fields.
    fn speech_memory(&self) -> SpeechMemory {
        SpeechMemory {
            rng: self.rng.clone(),
            clip: self.last_speech_clip.clone(),
            recent: self.speech_recent_clips.clone(),
            index: self.speech_gesture_index,
            body_beat: self.speech_last_body_beat,
            tilt: self.speech_last_tilt,
            turn: self.speech_last_turn,
            body: self.speech_previous_body.clone(),
            seconds: self.speech_seconds,
            emphasis_at: self.speech_emphasis_at,
        }
    }

    /// Put a saved snapshot back into the coordinator.
    /// It takes the values from a saved snapshot and puts them into the coordinator’s corresponding fields
    fn restore_speech_memory(&mut self, memory: SpeechMemory) {
        self.rng = memory.rng;
        self.last_speech_clip = memory.clip;
        self.speech_recent_clips = memory.recent;
        self.speech_gesture_index = memory.index;
        self.speech_last_body_beat = memory.body_beat;
        self.speech_last_tilt = memory.tilt;
        self.speech_last_turn = memory.turn;
        self.speech_previous_body = memory.body;
        self.speech_seconds = memory.seconds;
        self.speech_emphasis_at = memory.emphasis_at;
    }

    /// Prepare a future speech plan, then restore the coordinator’s existing history.
    fn plan_speech(
        &mut self,
        analysis: &SpeechAnalysis,
        motions: &MotionLibrary,
        anchor: &JointPositions,
    ) -> Result<(MotionDefinition, Vec<(usize, SpeechMemory)>)> {
        // first save the current state so we can restore it after planning
        // this saves the starting point before the composer changes anything
        let performed = self.speech_memory();
        // temporarily move the existing checkpoints out of the coordinator
        let committed = std::mem::take(&mut self.speech_checkpoints);
        // compose the speech performance
        // The composer chooses gestures, timing, directions, and body shapes. While doing so, it updates the coordinator’s planning fields as though it were progressing through those future gestures.
        // It also builds a new checkpoint list containing snapshots of the history after each planned gesture.
        // At this moment, the coordinator temporarily contains the candidate future’s history.
        // But no movement has been started by plan_speech().
        let result = self.compose_speech_performance(analysis, motions, anchor);
        // retrieve the new checkpoints and put the old ones back.
        let checkpoints = std::mem::replace(&mut self.speech_checkpoints, committed);
        // restore the original speech history
        self.restore_speech_memory(performed);
        // return candidate plan and its checkpoints if composition succeeds
        result.map(|motion| (motion, checkpoints))
    }

    /// Updates speech history to match the gesture checkpoints that execution
    /// has passed.
    ///
    /// First, find the active movement and confirm its run ID matches our speech
    /// movement. If there is no matching movement or no keyframe index, leave
    /// the history unchanged.
    ///
    /// Count the checkpoints before the current keyframe. Each checkpoint follows
    /// a gesture's body movement and optional hold. Being on that keyframe is
    /// not enough; execution must have moved beyond it.
    ///
    /// Restore the latest passed checkpoint's memory and remove all consumed
    /// checkpoints so they cannot be applied again. Earlier snapshots do not
    /// need restoring separately because the latest one includes their history.
    ///
    /// Returns true if history advanced, or false if nothing changed.
    /// Progress comes from the movement timeline; this method does not separately
    /// check whether the servos physically reached each intermediate pose.
    fn advance_speech_memory<D: RuntimeDriver>(&mut self, core: &RuntimeCore<D>) -> bool {
        let Some(motion) = core
            .snapshot()
            .motion
            .as_ref()
            .filter(|m| Some(m.run_id) == self.speech_motion_run_id)
        else {
            return false;
        };
        let Some(index) = motion.keyframe_index else {
            return false;
        };
        let count = self
            .speech_checkpoints
            .iter()
            .take_while(|(at, _)| *at < index)
            .count();
        if count == 0 {
            return false;
        }
        let memory = self.speech_checkpoints[count - 1].1.clone();
        self.speech_checkpoints.drain(..count);
        self.restore_speech_memory(memory);
        true
    }

    /// Starts and updates character movement while speech audio is playing.
    ///
    /// Wait until audio analysis and the current audio frame are available.
    /// Then update speech history from any gesture checkpoints already passed.
    ///
    /// If a speech movement is tracked, check whether its remaining plan needs
    /// updating. More audio can extend a plan that is nearing its end. A stream
    /// ending also requires an update, even if no extra audio arrived.
    ///
    /// Normally, wait until a gesture's body movement and hold have passed before
    /// replacing the remaining plan. If the stream ends with very little audio
    /// left, switch directly to a return to the anchor.
    ///
    /// Build the replacement from the remaining audio. Ask the runtime to continue
    /// from the existing movement's commanded position and velocity, keeping the
    /// same run ID and anchor. Install the new checkpoints only after it succeeds.
    ///
    /// If the tracked movement is still active, leave it running. Otherwise, clear
    /// its tracking and check whether this utterance still needs a startup attempt.
    /// With an anchor and available movement, prepare a fresh performance. An
    /// executing idle or thinking movement can hand over to speech; otherwise,
    /// start a new movement from holding.
    ///
    /// Record an attempted start so later ticks do not repeatedly launch it.
    /// Planning or movement failures leave audio playback to continue independently.
    fn tick_speaking<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
        analysis: Option<&SpeechAnalysis>,
        frame: Option<usize>,
    ) {
        let (Some(analysis), Some(frame)) = (analysis, frame) else {
            return;
        };
        let elapsed = frame as f64 * 0.020;
        let gesture_finished = self.advance_speech_memory(core);
        if let Some(run_id) = self.speech_motion_run_id {
            let finalizing = self.speech_plan_streaming && !analysis.streaming;
            let late_end = finalizing && analysis.duration_seconds - elapsed <= 0.9;
            if late_end
                || (gesture_finished
                    && (finalizing
                        || (analysis.duration_seconds > self.speech_planned_until
                            && self.speech_planned_until - elapsed < 1.5)))
            {
                if let Some(anchor) = self.status.active_anchor.clone() {
                    let tail = SpeechAnalysis {
                        rms_20ms: analysis.rms_20ms.iter().skip(frame).copied().collect(),
                        phrase_peaks: analysis
                            .phrase_peaks
                            .iter()
                            .filter_map(|peak| peak.checked_sub(frame))
                            .collect(),
                        quiet_regions: analysis
                            .quiet_regions
                            .iter()
                            .filter_map(|(start, end)| {
                                (*end > frame).then_some((
                                    start.saturating_sub(frame),
                                    end.saturating_sub(frame),
                                ))
                            })
                            .collect(),
                        duration_seconds: (analysis.duration_seconds - elapsed).max(0.0),
                        streaming: analysis.streaming,
                    };
                    // Do not introduce a new minimum-length gesture at a late EOF.
                    let settle_only = !tail.streaming && tail.duration_seconds <= 0.9;
                    let performance = if settle_only {
                        Ok((speech_settle_motion(), Vec::new()))
                    } else {
                        self.plan_speech(&tail, core.motions(), &anchor)
                    };
                    if let Ok((performance, checkpoints)) = performance {
                        if core
                            .extend_character_performance(run_id, performance, anchor, now)
                            .is_ok()
                        {
                            self.speech_checkpoints = checkpoints;
                            self.speech_planned_until = analysis.duration_seconds;
                            self.speech_plan_streaming = analysis.streaming;
                            if settle_only {
                                self.status.active_clip = Some("speak_settle".into());
                            }
                            if finalizing {
                                eprintln!(
                                    "{}",
                                    serde_json::json!({"event": "speech.motion_finalized", "motion_run_id": run_id, "remaining_ms": (tail.duration_seconds * 1000.0) as u64, "settle_only": settle_only})
                                );
                            } else {
                                eprintln!(
                                    "{}",
                                    serde_json::json!({
                                        "event": "speech.motion_extended", "motion_run_id": run_id,
                                        "completed_gestures": self.speech_gesture_index,
                                        "remaining_ms": (tail.duration_seconds * 1000.0) as u64,
                                    })
                                );
                            }
                        }
                    }
                }
            }
        }
        if let Some(run_id) = self.speech_motion_run_id {
            if core
                .snapshot()
                .motion
                .as_ref()
                .is_some_and(|motion| motion.run_id == run_id && !motion.state.is_terminal())
            {
                return;
            }
            // A run absent from the active slot cannot still own movement.
            // The bounded terminal history may already contain a newer run.
            self.speech_motion_run_id = None;
            self.status.active_clip = None;
        }
        if self.speech_motion_started {
            return;
        }
        let Some(anchor) = self.status.active_anchor.clone() else {
            return;
        };
        let prior = self
            .thinking_run
            .take()
            .or_else(|| self.active_idle_run_id.take());
        self.active_idle_category = None;
        if core.mode() != RuntimeMode::Holding && prior.is_none() {
            return;
        }
        self.speech_motion_started = true;
        self.speech_planned_until = analysis.duration_seconds;
        self.speech_plan_streaming = analysis.streaming;
        let Ok((performance, checkpoints)) = self.plan_speech(analysis, core.motions(), &anchor)
        else {
            return;
        };
        // Speech motion remains best-effort: playback is never failed because
        // a generated performance could not be compiled or started.
        let result = if let Some(run) = prior.filter(|run| {
            core.snapshot()
                .motion
                .as_ref()
                .is_some_and(|m| m.run_id == *run && m.state == MovementPhase::Executing)
        }) {
            core.extend_character_performance(run, performance, anchor, now)
        } else {
            core.play_generated_anchored_relative(performance, anchor, now)
        };
        if let Ok(run_id) = result {
            self.speech_checkpoints = checkpoints;
            self.speech_motion_run_id = Some(run_id);
            self.status.active_clip = Some("speaking_performance".into());
        }
    }

    /// Builds a speech movement plan from the available audio, authored motion
    /// shapes, and the character's anchor.
    ///
    /// First, work out the time available for gestures and reserve time for a
    /// final return to the anchor. Streaming audio gets a provisional time budget
    /// because more audio may still arrive.
    ///
    /// Choose gestures using nearby audio peaks, recent clip history, and seeded
    /// random variation. Space out emphasis and stronger body accents, and vary
    /// head direction, body shape, and timing to avoid repetitive movement.
    ///
    /// When a suitable quiet interval appears, arrange for the gesture to reach
    /// its pose and hold there. Keep that phrase pose during the pause rather
    /// than returning to the anchor between sentences.
    ///
    /// Turn each planned gesture into two keyframes: the head leads while the
    /// body keeps its previous shape, then the body follows. During the follow,
    /// the head can begin blending toward the next gesture unless a hold is needed.
    ///
    /// Save a speech-memory checkpoint at each body-follow keyframe, then append
    /// one final zero-offset settle that returns the movement to its anchor.
    ///
    /// This method advances planning history and writes future checkpoints while
    /// composing. plan_speech() saves and restores the existing history around
    /// this work so unperformed gestures are not remembered as already reached.
    ///
    /// Returns the movement definition or a planning error. The runtime later
    /// compiles and executes the definition using the calibrated joint limits.
    fn compose_speech_performance(
        &mut self,
        analysis: &SpeechAnalysis,
        motions: &MotionLibrary,
        anchor: &JointPositions,
    ) -> Result<MotionDefinition> {
        let style = MotionStyle::named("speaking_emphatic")?;
        let performance_seconds = (analysis.duration_seconds
            + if analysis.streaming {
                1.5
            } else {
                -SPEECH_END_LEAD_SECONDS
            })
        .max(0.9);
        let authored_budget = performance_seconds * style.tempo;
        let settle_budget = (SPEECH_FINAL_SETTLE_SECONDS * style.tempo)
            .min(authored_budget * 0.35)
            .max(0.16);
        let active_budget = (authored_budget - settle_budget).max(0.24);
        let mut authored_seconds = 0.0;
        let mut drawings = Vec::new();
        let mut peak_cursor = 0;
        let mut gesture_index = self.speech_gesture_index;
        let mut last_tilt_direction = self.speech_last_tilt;
        let mut last_turn_direction = self.speech_last_turn;
        let mut last_body_beat = self.speech_last_body_beat;
        let base_seconds = self.speech_seconds;
        let initial_body = self.speech_previous_body.clone();
        let maximum_rms = analysis.rms_20ms.iter().copied().fold(0.0_f64, f64::max);

        while active_budget - authored_seconds > 0.22 {
            let current_frame = ((authored_seconds / style.tempo) / 0.020).round() as usize;
            while analysis
                .phrase_peaks
                .get(peak_cursor)
                .is_some_and(|peak| *peak + 10 < current_frame)
            {
                peak_cursor += 1;
            }
            let phrase_peak = analysis
                .phrase_peaks
                .get(peak_cursor)
                .copied()
                .filter(|peak| *peak <= current_frame + SPEECH_PEAK_LOOKAHEAD_FRAMES);
            let gesture_seconds = base_seconds + authored_seconds / style.tempo;
            let emphasis = phrase_peak.is_some()
                && self
                    .speech_emphasis_at
                    .is_none_or(|last| gesture_seconds - last >= SPEECH_EMPHASIS_INTERVAL_SECONDS);
            if emphasis {
                peak_cursor += 1;
            }
            let energy_ratio = phrase_peak
                .and_then(|peak| analysis.rms_20ms.get(peak).copied())
                .map(|rms| rms / maximum_rms.max(f64::EPSILON))
                .unwrap_or(0.0);
            let body_beat_available = last_body_beat
                .is_none_or(|last| gesture_index - last >= SPEECH_BODY_BEAT_INTERVAL_DRAWINGS);
            let body_beat = emphasis
                && energy_ratio >= 0.72
                && body_beat_available
                && self.last_speech_clip.as_deref() != Some("speak_explanatory_lean");
            let clip = if body_beat {
                "speak_explanatory_lean".to_owned()
            } else {
                self.choose_speech_clip(emphasis)
            };
            let definition = motions.motion(&clip)?;
            let nominal_seconds = if emphasis { 0.90 } else { 1.35 }
                * SPEECH_GESTURE_DURATION_SCALE
                * self.rng.range(0.90, 1.10);
            let remaining = active_budget - authored_seconds;
            let fit = (remaining / nominal_seconds).min(1.0);
            if fit < 0.24 && !drawings.is_empty() {
                break;
            }

            let source = &definition.keyframes[0].target;
            let head_scale = if emphasis {
                self.rng.range(1.05, 1.24)
            } else {
                self.rng.range(0.88, 1.10)
            };
            let mut head_target = JointPositions::new();
            // Sustained eyelines and neutral passages break compulsory left/right sway.
            let choices = if last_tilt_direction == 0.0 {
                [-1.0, 0.0, 1.0, 1.0]
            } else {
                [
                    last_tilt_direction,
                    last_tilt_direction,
                    0.0,
                    -last_tilt_direction,
                ]
            };
            let direction = choices[self.rng.index(choices.len())];
            last_tilt_direction = direction;
            let roll = source
                .get("head_roll_joint")
                .copied()
                .unwrap_or(0.060)
                .abs();
            head_target.insert("head_roll_joint".into(), roll * direction * head_scale);
            let head_pitch = source.get("head_pitch_joint").copied().unwrap_or_else(|| {
                if gesture_index % 3 == 2 {
                    -self.rng.range(0.030, 0.045)
                } else {
                    self.rng.range(0.040, 0.065)
                }
            });
            head_target.insert("head_pitch_joint".into(), head_pitch * head_scale);

            let anchor_yaw = anchor.get("base_yaw_joint").copied().unwrap_or(0.0);
            let turn_direction =
                self.choose_speech_turn_direction(anchor_yaw, last_turn_direction, emphasis);
            last_turn_direction = turn_direction;
            if turn_direction != 0 {
                let magnitude = if emphasis {
                    self.rng.range(0.070, 0.110)
                } else {
                    self.rng.range(0.045, 0.085)
                };
                head_target.insert("base_yaw_joint".into(), turn_direction as f64 * magnitude);
            }

            let body_scale = if body_beat {
                self.rng.range(0.78, 0.96)
            } else {
                self.rng.range(0.32, 0.48)
            };
            let source_shoulder = source
                .get("shoulder_pitch_joint")
                .copied()
                .unwrap_or_else(|| self.rng.range(-0.035, 0.035));
            let source_elbow = source
                .get("elbow_pitch_joint")
                .copied()
                .unwrap_or(-source_shoulder.signum() * self.rng.range(0.035, 0.050));
            let body_target = JointPositions::from([
                ("shoulder_pitch_joint".into(), source_shoulder * body_scale),
                ("elbow_pitch_joint".into(), source_elbow * body_scale),
            ]);

            let mut duration_seconds = nominal_seconds * fit;
            let start_seconds = authored_seconds / style.tempo;
            // Reach a phrase pose at a substantial quiet interval, then hold it.
            // Do not return to the anchor between sentences.
            let pause = analysis.quiet_regions.iter().find_map(|(start, end)| {
                let start = *start as f64 * 0.020;
                let end = *end as f64 * 0.020;
                (end - start >= 0.4
                    && start >= start_seconds + 0.3
                    && start <= start_seconds + duration_seconds / style.tempo)
                    .then_some((start, end))
            });
            let hold_seconds = if let Some((start, end)) = pause {
                duration_seconds = (start - start_seconds) * style.tempo;
                ((end - start).min(1.2) * style.tempo).min((remaining - duration_seconds).max(0.0))
            } else {
                0.0
            };
            authored_seconds += duration_seconds + hold_seconds;
            if body_beat {
                last_body_beat = Some(gesture_index);
            }
            if emphasis {
                self.speech_emphasis_at = Some(gesture_seconds);
            }
            self.last_speech_clip = Some(clip.clone());
            self.speech_recent_clips.push(clip.clone());
            if self.speech_recent_clips.len() > 2 {
                self.speech_recent_clips.remove(0);
            }
            gesture_index += 1;
            let lead_fraction = self.rng.range(0.64, 0.74);
            self.speech_gesture_index = gesture_index;
            self.speech_last_body_beat = last_body_beat;
            self.speech_last_tilt = last_tilt_direction;
            self.speech_last_turn = last_turn_direction;
            self.speech_seconds = base_seconds + authored_seconds / style.tempo;
            self.speech_previous_body = body_target.clone();
            drawings.push(PlannedSpeechDrawing {
                clip,
                head_target,
                body_target,
                duration_seconds,
                body_beat,
                lead_fraction,
                hold_seconds,
                memory: self.speech_memory(),
            });
        }

        if drawings.is_empty() {
            return Err(Error::Runtime(
                "Speech performance could not allocate an expressive keyframe.".into(),
            ));
        }

        // Each phrase is staged in two drawings: the head leads the thought,
        // then the shoulder and elbow follow while the head already begins the
        // next arc. This provides anticipation and overlapping action without
        // giving the secondary body motion a competing rhythmic oscillator.
        let mut keyframes = Vec::with_capacity(drawings.len() * 2 + 1);
        let mut previous_body = initial_body;
        self.speech_checkpoints.clear();
        for (index, drawing) in drawings.iter().enumerate() {
            let lead_fraction = drawing.lead_fraction;
            keyframes.push(MotionKeyframe {
                pose_name: None,
                target: merge_speech_layers(&drawing.head_target, &previous_body),
                duration_seconds: drawing.duration_seconds * lead_fraction,
                arrival: KeyframeArrival::Through,
                hold_seconds: 0.0,
                marker: Some(format!("gesture_{index}_{}", drawing.clip)),
            });
            let next_head = drawings
                .get(index + 1)
                .map(|next| next.head_target.clone())
                .unwrap_or_default();
            let following_head = if drawing.hold_seconds > 0.0 {
                drawing.head_target.clone()
            } else {
                blend_speech_head(&drawing.head_target, &next_head, 0.18)
            };
            keyframes.push(MotionKeyframe {
                pose_name: None,
                target: merge_speech_layers(&following_head, &drawing.body_target),
                duration_seconds: drawing.duration_seconds * (1.0 - lead_fraction),
                arrival: if drawing.hold_seconds > 0.0 {
                    KeyframeArrival::Settle
                } else {
                    KeyframeArrival::Through
                },
                hold_seconds: drawing.hold_seconds,
                marker: Some(if drawing.body_beat {
                    format!("body_beat_{index}")
                } else {
                    format!("body_follow_{index}")
                }),
            });
            previous_body = drawing.body_target.clone();
            self.speech_checkpoints
                .push((keyframes.len() - 1, drawing.memory.clone()));
        }
        self.speech_gesture_index = gesture_index;
        self.speech_last_body_beat = last_body_beat;
        self.speech_last_tilt = last_tilt_direction;
        self.speech_last_turn = last_turn_direction;
        self.speech_previous_body = previous_body;
        keyframes.push(MotionKeyframe {
            pose_name: None,
            target: JointPositions::new(),
            duration_seconds: (authored_budget - authored_seconds).max(0.12),
            arrival: KeyframeArrival::Settle,
            hold_seconds: 0.0,
            marker: Some("speech_settled".into()),
        });
        Ok(MotionDefinition {
            name: "speaking_performance".into(),
            description: "Utterance-length continuous speaking performance.".into(),
            space: MotionSpace::AnchorRelative,
            style,
            return_to_anchor: true,
            keyframes,
        })
    }

    fn choose_speech_turn_direction(
        &mut self,
        anchor_yaw: f64,
        previous: i8,
        emphasis: bool,
    ) -> i8 {
        let weighted: &[i8] = if emphasis {
            &[-1, 0, 0, 0, 1]
        } else {
            &[-1, -1, 0, 1, 1]
        };
        let candidates: Vec<i8> = weighted
            .iter()
            .copied()
            .filter(|direction| *direction != previous)
            .filter(|direction| !(anchor_yaw >= 0.75 && *direction > 0))
            .filter(|direction| !(anchor_yaw <= -0.75 && *direction < 0))
            .collect();
        candidates[self.rng.index(candidates.len())]
    }

    fn choose_speech_clip(&mut self, emphasis: bool) -> String {
        let weighted: &[(&str, u64)] = if emphasis {
            &[
                ("speak_emphasis_nod", 3),
                ("speak_reflective_tilt", 2),
                ("speak_calm_sway", 1),
            ]
        } else {
            &[
                ("speak_calm_sway", 5),
                ("speak_explanatory_lean", 2),
                ("speak_reflective_tilt", 3),
            ]
        };
        let candidates: Vec<(&str, u64)> = weighted
            .iter()
            .copied()
            .filter(|(clip, _)| self.last_speech_clip.as_deref() != Some(*clip))
            .collect();
        // Penalize recent shapes rather than forcing a deterministic three-clip cycle.
        let candidates: Vec<_> = candidates
            .into_iter()
            .map(|(clip, weight)| {
                let recent = self
                    .speech_recent_clips
                    .iter()
                    .any(|previous| previous == clip);
                (clip, if recent { 1 } else { weight * 3 })
            })
            .collect();
        let total_weight: u64 = candidates.iter().map(|(_, weight)| weight).sum();
        let mut ticket = self.rng.next() % total_weight;
        for (clip, weight) in candidates {
            if ticket < weight {
                return clip.to_owned();
            }
            ticket -= weight;
        }
        unreachable!("positive speech weights always select a candidate")
    }

    fn capture_anchor<D: RuntimeDriver>(&mut self, core: &RuntimeCore<D>) {
        self.status.active_anchor = Some(
            core.snapshot()
                .joints
                .iter()
                .map(|joint| (joint.name.clone(), joint.position))
                .collect(),
        );
    }

    fn idle_state(&self) -> CharacterState {
        if self
            .status
            .active_anchor
            .as_ref()
            .and_then(|anchor| anchor.get("base_yaw_joint"))
            .is_some_and(|yaw| yaw.abs() < 0.05)
        {
            CharacterState::HomeIdle
        } else {
            CharacterState::PoseIdle
        }
    }

    fn reset_timers(&mut self, now: f64) {
        self.next_micro_at = now
            + self
                .rng
                .range(MICRO_IDLE_MIN_SECONDS, MICRO_IDLE_MAX_SECONDS);
        self.next_large_at = now
            + self
                .rng
                .range(LARGE_IDLE_MIN_SECONDS, LARGE_IDLE_MAX_SECONDS);
        self.update_next_idle_category();
    }

    fn reschedule_idle(&mut self, category: NextIdleCategory, now: f64) {
        match category {
            NextIdleCategory::Micro => {
                self.next_micro_at = now
                    + self
                        .rng
                        .range(MICRO_IDLE_MIN_SECONDS, MICRO_IDLE_MAX_SECONDS);
            }
            NextIdleCategory::Large => {
                self.next_large_at = now
                    + self
                        .rng
                        .range(LARGE_IDLE_MIN_SECONDS, LARGE_IDLE_MAX_SECONDS);
            }
        }
        self.update_next_idle_category();
    }

    /// Updates the next idle category based on the current idle times.
    fn update_next_idle_category(&mut self) {
        self.status.next_idle_category = Some(if self.next_micro_at <= self.next_large_at {
            NextIdleCategory::Micro
        } else {
            NextIdleCategory::Large
        });
    }

    fn finish_stop(&mut self) {
        self.thinking_run = None;
        self.clear_attention();
        self.status = CharacterStatus {
            enabled: false,
            state: CharacterState::Off,
            active_anchor: None,
            active_clip: None,
            next_idle_category: None,
        };
        self.active_idle_run_id = None;
        self.active_idle_category = None;
        self.starting_run_id = None;
        self.foreground_pending = false;
        self.foreground_scene_run_id = None;
        self.speech_motion_run_id = None;
        self.speech_motion_started = false;
    }
}

/// Returns a motion definition for returning to the previous anchor after attention.
fn attention_return_motion() -> MotionDefinition {
    let mut motion = speech_settle_motion();
    motion.name = "attention_return".into();
    motion.style = MotionStyle::named("return_home").expect("built-in return style exists");
    motion.description = "Return from conversational attention to the previous anchor.".into();
    motion.keyframes[0].duration_seconds = 1.2;
    motion.keyframes[0].marker = Some("attention_returned".into());
    motion
}

/// Merges the head and body target positions into a single joint positions map.
fn merge_speech_layers(
    head_target: &JointPositions,
    body_target: &JointPositions,
) -> JointPositions {
    let mut target = head_target.clone();
    target.extend(body_target.clone());
    target
}

/// Blends the current head pose toward the next head pose over a given lookahead duration.
fn blend_speech_head(
    current: &JointPositions,
    next: &JointPositions,
    lookahead: f64,
) -> JointPositions {
    ["base_yaw_joint", "head_roll_joint", "head_pitch_joint"]
        .into_iter()
        .filter_map(|joint| {
            let current_value = current.get(joint).copied().unwrap_or(0.0);
            let next_value = next.get(joint).copied().unwrap_or(0.0);
            let blended = current_value * (1.0 - lookahead) + next_value * lookahead;
            (blended.abs() > f64::EPSILON).then(|| (joint.to_owned(), blended))
        })
        .collect()
}

/// Returns a motion definition for the character's thinking movement.
fn thinking_motion() -> MotionDefinition {
    // Radians relative to the conversational anchor. The existing thinking
    // style scales these drawings; calibration may uniformly reduce them.
    let drawings = [
        // marker, seconds, head roll, head pitch, yaw, shoulder, elbow
        ("think_prepare", 0.22, -0.035, -0.025, -0.012, -0.015, 0.018),
        ("think_head_lead", 0.65, 0.24, 0.16, 0.085, -0.015, 0.018),
        ("think_body_follow", 0.40, 0.21, 0.13, 0.12, -0.028, 0.035),
        (
            "think_counter_lead",
            0.90,
            -0.18,
            -0.10,
            -0.07,
            -0.028,
            0.035,
        ),
        (
            "think_counter_follow",
            0.40,
            -0.15,
            -0.07,
            -0.09,
            0.020,
            -0.025,
        ),
        ("think_resolve", 0.75, 0.09, 0.065, 0.035, 0.0, 0.0),
    ];
    let mut motion = MotionDefinition {
        name: "thinking_head".into(),
        description: "Readable head-led thought with anticipation and delayed body support.".into(),
        space: MotionSpace::AnchorRelative,
        style: MotionStyle::named("thinking").expect("built-in thinking style exists"),
        return_to_anchor: true,
        keyframes: drawings
            .into_iter()
            .map(
                |(marker, duration, roll, pitch, yaw, shoulder, elbow)| MotionKeyframe {
                    pose_name: None,
                    target: [
                        ("head_roll_joint".into(), roll),
                        ("head_pitch_joint".into(), pitch),
                        ("base_yaw_joint".into(), yaw),
                        ("shoulder_pitch_joint".into(), shoulder),
                        ("elbow_pitch_joint".into(), elbow),
                    ]
                    .into(),
                    duration_seconds: duration,
                    arrival: KeyframeArrival::Through,
                    hold_seconds: 0.0,
                    marker: Some(marker.into()),
                },
            )
            .collect(),
    };
    motion.keyframes.push(MotionKeyframe {
        pose_name: None,
        target: JointPositions::new(),
        duration_seconds: 0.75,
        arrival: KeyframeArrival::Settle,
        hold_seconds: 0.0,
        marker: Some("thinking_settled".into()),
    });
    motion
}

/// Returns a motion definition for the character's speech settling movement.
fn speech_settle_motion() -> MotionDefinition {
    MotionDefinition {
        name: "speak_settle".into(),
        description: "Blend an interrupted speaking performance back to its anchor.".into(),
        space: MotionSpace::AnchorRelative,
        style: MotionStyle::named("speaking_calm").expect("built-in speaking style exists"),
        return_to_anchor: true,
        keyframes: vec![MotionKeyframe {
            pose_name: None,
            target: JointPositions::new(),
            duration_seconds: 0.42,
            arrival: KeyframeArrival::Settle,
            hold_seconds: 0.0,
            marker: Some("speech_settled".into()),
        }],
    }
}

/// Checks the response from a character operation and returns the parsed JSON value.
fn checked(response: String) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(&response)?;
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        Ok(value)
    } else {
        Err(Error::InvalidState(
            value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("character operation failed")
                .to_owned(),
        ))
    }
}

/// Returns the terminal phase of a motion, if one exists.
fn terminal_phase<D: RuntimeDriver>(core: &RuntimeCore<D>, run_id: u64) -> Option<MovementPhase> {
    [
        core.snapshot().motion.as_ref(),
        core.snapshot().last_motion.as_ref(),
    ]
    .into_iter()
    .flatten()
    .find(|motion| motion.run_id == run_id && motion.state.is_terminal())
    .map(|motion| motion.state)
}

/// Returns the closest pose profile for a given anchor position.
/// uses the sum of squared differences between anchor and pose positions to find the closest profile.
fn closest_pose_profile<D: RuntimeDriver>(
    core: &RuntimeCore<D>,
    anchor: &JointPositions,
) -> Option<String> {
    core.poses()
        .names()
        .into_iter()
        .filter_map(|name| {
            let definition = core.poses().definition(&name).ok()?;
            let distance: f64 = definition
                .positions
                .iter()
                .map(|(joint, value)| (anchor[joint] - value).powi(2))
                .sum();
            Some((distance, definition.idle_profile.clone()))
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .and_then(|(_, profile)| profile)
}

/// helper method to determine lighting effect for orion based on its current anchor
/// it does this by comparing the anchor with known poses from the pose library
/// selecting the closest pose and returning the lighting effect associated with the closes pose
/// if no pose is found, returns `None`
fn closest_pose_default_lighting<D: RuntimeDriver>(
    core: &RuntimeCore<D>,
    anchor: &JointPositions,
) -> Option<String> {
    core.poses()
        .names()
        .into_iter()
        .filter_map(|name| {
            let definition = core.poses().definition(&name).ok()?;
            // we calc how close the anchor is to the pose by summing the squared differences of joint positions
            // For every joint in the pose:
            // 1. Read the anchor’s angle for that joint.
            // 2. Subtract the candidate pose’s angle.
            // 3. Square the difference.
            // 4. Add the squared differences across all joints.
            // this allows us give the pose a score based on how far its joint angles differ from the anchor
            let distance: f64 = definition
                .positions
                .iter()
                .map(|(joint, value)| (anchor[joint] - value).powi(2))
                .sum();
            Some((distance, definition.default_lighting.clone()))
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .and_then(|(_, lighting)| lighting)
}

/// Returns whether the closest pose is a shutdown-only pose.
fn closest_pose_is_shutdown_only<D: RuntimeDriver>(
    core: &RuntimeCore<D>,
    anchor: &JointPositions,
) -> bool {
    core.poses()
        .names()
        .into_iter()
        .filter_map(|name| {
            let definition = core.poses().definition(&name).ok()?;
            let distance: f64 = definition
                .positions
                .iter()
                .map(|(joint, value)| (anchor[joint] - value).powi(2))
                .sum();
            let shutdown_only = definition
                .tags
                .iter()
                .any(|tag| matches!(tag.as_str(), "shutdown_only" | "mechanical"));
            Some((distance, shutdown_only))
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .is_some_and(|(_, shutdown_only)| shutdown_only)
}

#[derive(Clone, Debug)]
struct SeededRandom {
    state: u64,
}
impl SeededRandom {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }
    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
    fn range(&mut self, lower: f64, upper: f64) -> f64 {
        lower + (upper - lower) * (self.next() as f64 / u64::MAX as f64)
    }
    fn index(&mut self, length: usize) -> usize {
        (self.next() % length as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use crate::ORION_JOINT_NAMES;
    use crate::control::state::JointState;
    use crate::devices::sts3215::driver::JointLimit;
    use crate::motion::library::{MotionLibrary, MotionSequence};
    use crate::motion::pose::PoseLibrary;

    use super::*;

    struct CharacterTestDriver;

    impl RuntimeDriver for CharacterTestDriver {
        fn apply_servo_profile(&mut self) -> Result<()> {
            Ok(())
        }
        fn activate(&mut self) -> Result<Vec<JointState>> {
            Ok(self.read()?)
        }
        fn deactivate(&mut self) -> Result<()> {
            Ok(())
        }
        fn read(&mut self) -> Result<Vec<JointState>> {
            Ok(ORION_JOINT_NAMES
                .iter()
                .map(|name| JointState {
                    name: (*name).to_owned(),
                    position: 0.0,
                    velocity: 0.0,
                    current_ma: 0.0,
                    voltage_v: 7.4,
                    temperature_c: 25.0,
                    status: 0,
                })
                .collect())
        }
        fn write(&mut self, _positions_radians: &JointPositions) -> Result<()> {
            Ok(())
        }
        fn joint_limits(&self) -> Result<Vec<JointLimit>> {
            Ok(ORION_JOINT_NAMES
                .iter()
                .map(|name| JointLimit {
                    name: (*name).to_owned(),
                    lower_rad: -3.0,
                    upper_rad: 3.0,
                })
                .collect())
        }
        fn validate_positions(&self, _positions_radians: &JointPositions) -> Result<()> {
            Ok(())
        }
        fn clamp_positions_to_safe_range(
            &self,
            positions_radians: &JointPositions,
        ) -> Result<JointPositions> {
            Ok(positions_radians.clone())
        }
    }

    fn core() -> RuntimeCore<CharacterTestDriver> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        RuntimeCore::new(CharacterTestDriver, poses, motions).unwrap()
    }

    struct FollowingDriver {
        positions: JointPositions,
    }
    impl RuntimeDriver for FollowingDriver {
        fn apply_servo_profile(&mut self) -> Result<()> {
            Ok(())
        }
        fn activate(&mut self) -> Result<Vec<JointState>> {
            self.read()
        }
        fn deactivate(&mut self) -> Result<()> {
            Ok(())
        }
        fn read(&mut self) -> Result<Vec<JointState>> {
            let mut states = CharacterTestDriver.read()?;
            for state in &mut states {
                state.position = self.positions[&state.name];
            }
            Ok(states)
        }
        fn write(&mut self, positions: &JointPositions) -> Result<()> {
            self.positions = positions.clone();
            Ok(())
        }
        fn joint_limits(&self) -> Result<Vec<JointLimit>> {
            CharacterTestDriver.joint_limits()
        }
        fn validate_positions(&self, positions: &JointPositions) -> Result<()> {
            CharacterTestDriver.validate_positions(positions)
        }
        fn clamp_positions_to_safe_range(
            &self,
            positions: &JointPositions,
        ) -> Result<JointPositions> {
            Ok(positions.clone())
        }
    }

    fn following_core() -> RuntimeCore<FollowingDriver> {
        let source = core();
        let poses = source.poses().clone();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let positions = poses.pose("home").unwrap().clone();
        RuntimeCore::new(FollowingDriver { positions }, poses, motions).unwrap()
    }

    fn advance<D: RuntimeDriver>(
        character: &mut CharacterCoordinator,
        core: &mut RuntimeCore<D>,
        start: f64,
        end: f64,
    ) {
        for step in 1..=((end - start) * 50.0) as usize {
            let now = start + step as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, core, false, None, false, None, None)
                .unwrap();
        }
    }

    #[test]
    fn cancelled_and_timed_out_startup_never_become_home_idle() {
        for cancel in [false, true] {
            let mut core = core();
            let mut character = CharacterCoordinator::new(1);
            character.start(0.0, &mut core).unwrap();
            if cancel {
                checked(core.handle_command("stop", 0.1)).unwrap();
            }
            advance(&mut character, &mut core, 0.1, 20.1);
            assert!(!character.status.enabled);
            assert_eq!(character.status.state, CharacterState::Off);
            assert!(character.status.active_anchor.is_none());
        }
    }

    #[test]
    fn attention_holds_conversation_anchor_and_returns_after_neutral() {
        for side in ["left", "right"] {
            let mut core = following_core();
            let mut character = CharacterCoordinator::new(1);
            character.start(0.0, &mut core).unwrap();
            advance(&mut character, &mut core, 0.0, 3.0);
            let original = character.status.active_anchor.clone().unwrap();
            character.attend(side, 0.9, 3.0, &mut core).unwrap();
            assert_eq!(character.status.active_anchor.as_ref(), Some(&original));
            advance(&mut character, &mut core, 3.0, 6.0);
            let facing = character.status.active_anchor.clone().unwrap();
            assert!((facing["base_yaw_joint"].abs() - 0.35).abs() < 0.001);
            character.set_reaction("thinking", 6.0, &mut core).unwrap();
            advance(&mut character, &mut core, 6.0, 10.0);
            assert_eq!(character.status.active_anchor.as_ref(), Some(&facing));
            character.set_reaction("neutral", 10.0, &mut core).unwrap();
            advance(&mut character, &mut core, 10.0, 30.0);
            assert_eq!(character.status.active_anchor.as_ref(), Some(&original));
            assert!(character.attention.is_none());
        }
    }

    #[test]
    fn attention_rejects_low_confidence_and_off_and_yields_to_foreground() {
        let mut core = following_core();
        let mut character = CharacterCoordinator::new(1);
        assert!(character.attend("left", 0.9, 0.0, &mut core).is_err());
        character.start(0.0, &mut core).unwrap();
        advance(&mut character, &mut core, 0.0, 3.0);
        assert!(character.attend("left", 0.5, 3.0, &mut core).is_err());
        assert!(character.attend("right", f64::NAN, 3.0, &mut core).is_err());
        character.attend("left", 0.9, 3.0, &mut core).unwrap();
        let original = character.status.active_anchor.clone();
        checked(core.handle_command("stop", 3.1)).unwrap();
        advance(&mut character, &mut core, 3.1, 3.2);
        assert_eq!(character.status.active_anchor, original);
        assert!(character.attention.is_none());
        character.attend("right", 0.9, 3.2, &mut core).unwrap();
        character.note_foreground_started(3.3);
        assert!(character.attention.is_none());
    }

    #[test]
    fn seeded_schedule_stays_in_contract_ranges() {
        let mut random = SeededRandom::new(42);
        for _ in 0..100 {
            assert!((8.0..=20.0).contains(&random.range(8.0, 20.0)));
            assert!((35.0..=75.0).contains(&random.range(35.0, 75.0)));
        }
    }

    #[test]
    fn completing_one_idle_category_preserves_the_other_deadline() {
        let mut character = CharacterCoordinator::new(42);
        character.next_micro_at = 10.0;
        character.next_large_at = 40.0;

        character.reschedule_idle(NextIdleCategory::Micro, 12.0);
        assert_eq!(character.next_large_at, 40.0);
        assert!((20.0..=32.0).contains(&character.next_micro_at));

        let micro_deadline = character.next_micro_at;
        character.reschedule_idle(NextIdleCategory::Large, 41.0);
        assert_eq!(character.next_micro_at, micro_deadline);
        assert!((76.0..=116.0).contains(&character.next_large_at));
    }

    #[test]
    fn idle_timeout_is_recoverable_and_preserves_the_anchor() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();

        let anchor = core.poses().pose("home").unwrap().clone();
        let run_id = core
            .play_anchored_relative("idle_breathe", anchor.clone(), 0.0)
            .unwrap();
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.status.active_clip = Some("idle_breathe".into());
        character.active_idle_run_id = Some(run_id);
        character.active_idle_category = Some(NextIdleCategory::Micro);
        character.next_large_at = 40.0;

        let mut now = 0.0;
        while terminal_phase(&core, run_id).is_none() {
            now += 0.1;
            core.tick(now).unwrap();
            assert!(now < 30.0, "idle did not reach a terminal phase");
        }
        assert_eq!(terminal_phase(&core, run_id), Some(MovementPhase::TimedOut));

        character
            .tick(now, &mut core, false, None, false, None, None)
            .unwrap();

        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        assert_eq!(character.status.state, CharacterState::HomeIdle);
        assert!(character.active_idle_run_id.is_none());
        assert!(character.status.active_clip.is_none());
        assert!(
            (now + MICRO_IDLE_MIN_SECONDS..=now + MICRO_IDLE_MAX_SECONDS)
                .contains(&character.next_micro_at)
        );
        assert_eq!(character.next_large_at, 40.0);
    }

    #[test]
    fn priority_orders_scene_then_speech_then_reaction() {
        let core = &mut core();
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::Listening;
        character.status.active_anchor = Some(core.poses().pose("home").unwrap().clone());

        character
            .tick(1.0, core, true, None, true, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::ForegroundScene);

        character
            .tick(1.1, core, false, None, true, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::Speaking);

        character
            .tick(1.2, core, false, None, false, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::HomeIdle);
    }

    #[test]
    fn speech_preserves_the_immutable_idle_anchor() {
        let core = &mut core();
        let anchor = core.poses().pose("home").unwrap().clone();
        let measured: JointPositions = core
            .snapshot()
            .joints
            .iter()
            .map(|joint| (joint.name.clone(), joint.position))
            .collect();
        assert_ne!(anchor, measured);

        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.note_speech_started(1.0);
        character
            .tick(1.1, core, false, None, true, None, None)
            .unwrap();
        character
            .tick(1.2, core, false, None, false, None, None)
            .unwrap();

        assert_eq!(character.status.state, CharacterState::HomeIdle);
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        assert!(!character.foreground_pending);
    }

    #[test]
    fn every_speech_run_resets_its_continuous_performance() {
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.speech_motion_started = true;

        character.note_speech_started(1.0);

        assert!(!character.speech_motion_started);
    }

    #[test]
    fn utterance_length_speech_performance_flows_until_one_final_settle() {
        let core = core();
        let mut character = CharacterCoordinator::new(42);
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 1_000],
            quiet_regions: vec![(240, 255), (690, 705)],
            phrase_peaks: vec![60, 210, 360, 510, 660, 810, 940],
            duration_seconds: 20.0,
            streaming: false,
        };
        let anchor = core.poses().pose("home").unwrap().clone();
        let performance = character
            .compose_speech_performance(&analysis, core.motions(), &anchor)
            .unwrap();

        assert!(performance.keyframes.len() > 12);
        assert!(
            performance
                .keyframes
                .iter()
                .take(performance.keyframes.len() - 1)
                .all(|keyframe| keyframe.arrival == KeyframeArrival::Through)
        );
        assert_eq!(
            performance.keyframes.last().unwrap().arrival,
            KeyframeArrival::Settle
        );
        assert!(performance.keyframes.last().unwrap().target.is_empty());

        let markers = performance.markers();
        let gestures: Vec<_> = markers
            .iter()
            .filter(|marker| marker.starts_with("gesture_"))
            .collect();
        assert!(gestures.len() >= 8);
        assert!(
            gestures
                .windows(2)
                .all(|pair| pair[0].splitn(3, '_').nth(2) != pair[1].splitn(3, '_').nth(2))
        );

        let sequence = MotionSequence::new(&performance, anchor.clone()).unwrap();
        assert!((19.5..=20.1).contains(&sequence.duration_seconds()));
        for index in 0..sequence.keyframe_count() - 1 {
            let arrival = sequence.keyframe_arrival_time(index).unwrap();
            // A change of direction may have one instantaneous zero crossing,
            // but there must be commanded motion on both neighbouring 50 Hz
            // samples rather than a visible stopped plateau.
            let before = sequence.sample_state((arrival - 0.040).max(0.0)).unwrap();
            let after = sequence.sample_state(arrival + 0.040).unwrap();
            let before_speed: f64 = before.velocities.values().map(|value| value.abs()).sum();
            let after_speed: f64 = after.velocities.values().map(|value| value.abs()).sum();
            assert!(
                before_speed > 0.005,
                "stopped before speech keyframe {index}: {before_speed}"
            );
            assert!(
                after_speed > 0.005,
                "stopped after speech keyframe {index}: {after_speed}"
            );
        }
        assert_eq!(
            sequence.sample(sequence.duration_seconds()).unwrap(),
            anchor
        );
    }

    #[test]
    fn speech_pacing_spaces_emphasis_and_breaks_alternating_rolls() {
        let core = core();
        let anchor = core.poses().pose("home").unwrap().clone();
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 1500],
            quiet_regions: vec![],
            phrase_peaks: (0..1500).step_by(10).collect(),
            duration_seconds: 30.0,
            streaming: false,
        };
        let mut character = CharacterCoordinator::new(42);
        let (motion, checkpoints) = character
            .plan_speech(&analysis, core.motions(), &anchor)
            .unwrap();
        assert_eq!(
            character.speech_gesture_index, 0,
            "planning is not performance"
        );
        assert!(character.speech_recent_clips.is_empty());
        let rolls: Vec<_> = motion
            .keyframes
            .iter()
            .step_by(2)
            .filter_map(|k| k.target.get("head_roll_joint"))
            .collect();
        assert!(rolls.windows(2).any(|p| p[0].signum() == p[1].signum()));
        assert!(rolls.iter().any(|r| **r == 0.0));
        let emphasis: Vec<_> = checkpoints.iter().filter_map(|(_, m)| m.emphasis_at).fold(
            Vec::new(),
            |mut times, t| {
                if times.last() != Some(&t) {
                    times.push(t);
                }
                times
            },
        );
        assert!(emphasis.len() >= 2 && emphasis.len() <= 9);
        assert!(
            emphasis
                .windows(2)
                .all(|p| p[1] - p[0] >= SPEECH_EMPHASIS_INTERVAL_SECONDS)
        );
        let clips: Vec<_> = checkpoints
            .iter()
            .map(|(_, m)| m.clip.as_deref().unwrap())
            .collect();
        assert!(
            clips.windows(3).filter(|p| p[0] == p[2]).count() <= clips.len() / 4,
            "repeated ABAB motifs: {clips:?}"
        );
    }

    #[test]
    fn quiet_intervals_hold_the_phrase_pose_without_returning_home() {
        let core = core();
        let anchor = core.poses().pose("home").unwrap().clone();
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 600],
            quiet_regions: vec![(60, 110), (260, 310)],
            phrase_peaks: vec![],
            duration_seconds: 12.0,
            streaming: false,
        };
        let mut character = CharacterCoordinator::new(42);
        let motion = character
            .compose_speech_performance(&analysis, core.motions(), &anchor)
            .unwrap();
        let sequence = MotionSequence::new(&motion, anchor.clone()).unwrap();
        let holds: Vec<_> = motion
            .keyframes
            .iter()
            .enumerate()
            .filter(|(_, k)| k.hold_seconds > 0.0)
            .collect();
        assert!(!holds.is_empty());
        for (index, keyframe) in holds {
            assert!(!keyframe.target.is_empty());
            let arrival = sequence.keyframe_arrival_time(index).unwrap();
            let state = sequence.sample_state(arrival + 0.1).unwrap();
            assert!(state.velocities.values().all(|v| v.abs() < 1e-8));
            assert_ne!(state.positions, anchor);
        }
        assert_eq!(
            sequence.sample(sequence.duration_seconds()).unwrap(),
            anchor
        );
    }

    #[test]
    fn streaming_updates_preserve_first_gesture_and_commit_only_reached_history() {
        for chunk_frames in [20, 40] {
            let mut core = following_core();
            checked(core.handle_command("configure", 0.0)).unwrap();
            checked(core.handle_command("enable", 0.0)).unwrap();
            let mut character = CharacterCoordinator::new(42);
            character.status.enabled = true;
            character.status.active_anchor = Some(core.poses().pose("home").unwrap().clone());
            character.note_speech_started(0.0);
            let mut analysis = SpeechAnalysis {
                rms_20ms: vec![0.2; 150],
                quiet_regions: vec![],
                phrase_peaks: vec![],
                duration_seconds: 3.0,
                streaming: true,
            };
            character
                .tick(0.0, &mut core, false, None, true, Some(&analysis), Some(0))
                .unwrap();
            assert_eq!(character.speech_gesture_index, 0);
            let run = character.speech_motion_run_id;
            let first_clip = character.speech_checkpoints[0].1.clip.clone();
            for frame in 1..500 {
                let now = frame as f64 * 0.02;
                core.tick(now).unwrap();
                if frame % chunk_frames == 0 {
                    analysis.rms_20ms.extend(vec![0.2; chunk_frames]);
                    analysis.duration_seconds += chunk_frames as f64 * 0.02;
                }
                let before = core.snapshot().motion.as_ref().unwrap().progress;
                character
                    .tick(
                        now,
                        &mut core,
                        false,
                        None,
                        true,
                        Some(&analysis),
                        Some(frame),
                    )
                    .unwrap();
                assert_eq!(character.speech_motion_run_id, run);
                if frame < 30 {
                    assert_eq!(character.speech_gesture_index, 0);
                    assert_eq!(core.snapshot().motion.as_ref().unwrap().progress, before);
                }
                if character.speech_gesture_index == 1 {
                    assert_eq!(character.last_speech_clip, first_clip);
                }
            }
            assert!(character.speech_gesture_index >= 3);
        }
    }

    #[test]
    fn speech_performance_stages_head_first_and_keeps_body_beats_secondary() {
        let core = core();
        let anchor = core.poses().pose("home").unwrap().clone();
        let analysis = SpeechAnalysis {
            rms_20ms: (0..1_000)
                .map(|frame| 0.12 + (frame % 137) as f64 / 1_000.0)
                .collect(),
            quiet_regions: vec![],
            phrase_peaks: vec![60, 210, 360, 510, 660, 810, 940],
            duration_seconds: 20.0,
            streaming: false,
        };
        let mut character = CharacterCoordinator::new(42);
        let performance = character
            .compose_speech_performance(&analysis, core.motions(), &anchor)
            .unwrap();
        let active_keyframes = &performance.keyframes[..performance.keyframes.len() - 1];
        assert_eq!(active_keyframes.len() % 2, 0);

        let mut body_beat_indices = Vec::new();
        let mut ordinary_body_shapes = std::collections::BTreeSet::new();
        let mut turn_count = 0;
        for pair_start in (0..active_keyframes.len()).step_by(2) {
            let head_lead = &active_keyframes[pair_start];
            let body_follow = &active_keyframes[pair_start + 1];
            assert!(head_lead.marker.as_deref().unwrap().starts_with("gesture_"));
            let head_activity: f64 = ["base_yaw_joint", "head_roll_joint", "head_pitch_joint"]
                .into_iter()
                .map(|joint| head_lead.target.get(joint).copied().unwrap_or(0.0).abs())
                .sum();
            assert!(head_activity >= 0.08, "weak head lead: {head_activity}");
            turn_count += usize::from(
                head_lead
                    .target
                    .get("base_yaw_joint")
                    .is_some_and(|yaw| yaw.abs() >= 0.045),
            );

            let shoulder = body_follow
                .target
                .get("shoulder_pitch_joint")
                .copied()
                .unwrap_or(0.0)
                .abs();
            let elbow = body_follow
                .target
                .get("elbow_pitch_joint")
                .copied()
                .unwrap_or(0.0)
                .abs();
            let marker = body_follow.marker.as_deref().unwrap();
            if let Some(index) = marker.strip_prefix("body_beat_") {
                body_beat_indices.push(index.parse::<usize>().unwrap());
                assert!(shoulder >= 0.07, "body beat shoulder was not readable");
                assert!(elbow >= 0.09, "body beat elbow was not readable");
            } else {
                assert!(marker.starts_with("body_follow_"));
                assert!(shoulder <= 0.050, "ordinary shoulder competed with head");
                assert!(elbow <= 0.060, "ordinary elbow competed with head");
                assert!(
                    shoulder + elbow < head_activity,
                    "ordinary body action overtook the head lead"
                );
                ordinary_body_shapes
                    .insert(((shoulder * 10_000.0) as i64, (elbow * 10_000.0) as i64));
            }
        }

        let gesture_count = active_keyframes.len() / 2;
        assert!(turn_count >= gesture_count / 3);
        assert!(!body_beat_indices.is_empty());
        assert!(body_beat_indices.len() <= gesture_count.div_ceil(3));
        assert!(
            body_beat_indices
                .windows(2)
                .all(|pair| pair[1] - pair[0] >= 3)
        );
        assert!(ordinary_body_shapes.len() >= 4);
    }

    #[test]
    fn thinking_compiles_with_calibrated_limits_at_every_powered_anchor() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        for name in core.poses().names() {
            let definition = core.poses().definition(&name).unwrap();
            if !definition.tags.iter().any(|tag| tag == "idle_anchor") {
                continue;
            }
            let anchor = definition.positions.clone();
            let mut character = CharacterCoordinator::new(42);
            character.status.enabled = true;
            character.status.active_anchor = Some(anchor);
            character.set_reaction("thinking", 0.0, &mut core).unwrap();
            character
                .tick(0.0, &mut core, false, None, false, None, None)
                .unwrap();
            assert!(
                character.thinking_run.is_some(),
                "thinking failed at {name}"
            );
            character.set_reaction("neutral", 0.1, &mut core).unwrap();
            assert!(character.thinking_run.is_none());
            assert_ne!(character.status.state, CharacterState::Thinking);
        }
    }

    #[test]
    fn thinking_is_head_led_and_speech_replaces_its_commanded_spline() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.active_anchor = Some(anchor.clone());
        character.set_reaction("thinking", 0.0, &mut core).unwrap();
        character
            .tick(0.0, &mut core, false, None, false, None, None)
            .unwrap();
        let run = character
            .thinking_run
            .expect("thinking must compile and start");
        for frame in 1..50 {
            let now = frame as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
            assert_eq!(character.status.state, CharacterState::Thinking);
        }
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 200],
            quiet_regions: vec![],
            phrase_peaks: vec![40],
            duration_seconds: 4.0,
            streaming: true,
        };
        character.note_speech_started(1.0);
        character
            .tick(1.0, &mut core, false, None, true, Some(&analysis), Some(0))
            .unwrap();
        assert_eq!(character.speech_motion_run_id, Some(run));
        assert!(character.thinking_run.is_none());
        assert_eq!(character.status.active_anchor, Some(anchor));
        assert_eq!(character.status.state, CharacterState::Speaking);
        let motion = thinking_motion();
        assert_eq!(motion.style.name, "thinking");
        let frames = &motion.keyframes;
        let lead = &frames[1].target;
        let follow = &frames[2].target;
        assert!(frames[0].target["head_roll_joint"] * lead["head_roll_joint"] < 0.0);
        assert!(
            lead["head_roll_joint"] * motion.style.amplitude > 0.14,
            "dominant tilt must remain legible after artistic scaling"
        );
        for joint in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
            assert_eq!(
                frames[0].target[joint], lead[joint],
                "body waits for head lead"
            );
            assert_ne!(lead[joint], follow[joint], "body follows the head");
            assert!(
                frames
                    .iter()
                    .all(|frame| frame.target.get(joint).unwrap_or(&0.0).abs() <= 0.035)
            );
        }
        assert!(frames[..frames.len()-1].iter().all(|frame| frame.arrival == KeyframeArrival::Through && frame.hold_seconds == 0.0));
        assert_eq!(frames.last().unwrap().arrival, KeyframeArrival::Settle);
        assert!(frames.last().unwrap().target.is_empty());
    }

    #[test]
    fn streamed_speech_extends_without_resetting_anchor_or_settling_between_chunks() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let mut analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 110],
            quiet_regions: vec![],
            phrase_peaks: vec![40],
            duration_seconds: 2.2,
            streaming: true,
        };
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.note_speech_started(0.0);
        character
            .tick(0.0, &mut core, false, None, true, Some(&analysis), Some(0))
            .unwrap();
        let run = character.speech_motion_run_id.unwrap();
        for frame in 1..200 {
            let now = frame as f64 * 0.02;
            core.tick(now).unwrap();
            if frame % 40 == 0 {
                analysis.rms_20ms.extend(vec![0.2; 40]);
                analysis.duration_seconds += 0.8;
            }
            character
                .tick(
                    now,
                    &mut core,
                    false,
                    None,
                    true,
                    Some(&analysis),
                    Some(frame),
                )
                .unwrap();
            assert_eq!(character.speech_motion_run_id, Some(run));
            assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
            assert_eq!(character.status.state, CharacterState::Speaking);
        }
        character
            .tick(4.0, &mut core, false, None, false, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::Settling);
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("speak_settle")
        );
    }

    #[test]
    fn stream_end_without_new_audio_replans_once_and_late_end_only_settles() {
        for end_frame in [20, 95] {
            let mut core = following_core();
            checked(core.handle_command("configure", 0.0)).unwrap();
            checked(core.handle_command("enable", 0.0)).unwrap();
            let anchor = core.poses().pose("home").unwrap().clone();
            let mut analysis = SpeechAnalysis {
                rms_20ms: vec![0.2; 100],
                quiet_regions: vec![],
                phrase_peaks: vec![40],
                duration_seconds: 2.0,
                streaming: true,
            };
            let mut character = CharacterCoordinator::new(42);
            character.status.enabled = true;
            character.status.active_anchor = Some(anchor.clone());
            character.note_speech_started(0.0);
            character
                .tick(0.0, &mut core, false, None, true, Some(&analysis), Some(0))
                .unwrap();
            let run = character.speech_motion_run_id;
            for frame in 1..=end_frame {
                core.tick(frame as f64 * 0.02).unwrap();
                character
                    .tick(
                        frame as f64 * 0.02,
                        &mut core,
                        false,
                        None,
                        true,
                        Some(&analysis),
                        Some(frame),
                    )
                    .unwrap();
            }
            let before = character.speech_gesture_index;
            analysis.streaming = false;
            let mut now = end_frame as f64 * 0.02;
            character
                .tick(
                    now,
                    &mut core,
                    false,
                    None,
                    true,
                    Some(&analysis),
                    Some(end_frame),
                )
                .unwrap();
            assert_eq!(character.speech_motion_run_id, run);
            assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
            if end_frame == 95 {
                assert_eq!(
                    character.status.active_clip.as_deref(),
                    Some("speak_settle")
                );
                assert_eq!(character.speech_gesture_index, before);
            } else {
                assert_eq!(
                    character.speech_gesture_index, before,
                    "do not advance history or replace a mid-gesture plan"
                );
                assert!(character.speech_plan_streaming);
                for frame in end_frame + 1..100 {
                    let time = frame as f64 * 0.02;
                    now = time;
                    core.tick(time).unwrap();
                    character
                        .tick(
                            time,
                            &mut core,
                            false,
                            None,
                            true,
                            Some(&analysis),
                            Some(frame),
                        )
                        .unwrap();
                    if !character.speech_plan_streaming {
                        break;
                    }
                }
                assert!(
                    !character.speech_plan_streaming,
                    "end marker must finalize without another upload"
                );
            }
            let after = character.speech_gesture_index;
            let progress = core.snapshot().motion.as_ref().unwrap().progress;
            character
                .tick(
                    now,
                    &mut core,
                    false,
                    None,
                    true,
                    Some(&analysis),
                    Some((now / 0.02).round() as usize),
                )
                .unwrap();
            assert_eq!(core.snapshot().motion.as_ref().unwrap().progress, progress);
            assert_eq!(
                character.speech_gesture_index, after,
                "finalization must not repeat"
            );
            if end_frame == 95 {
                // Playback ending must not rewind a settle that EOF already installed.
                core.tick(now + 0.02).unwrap();
                let progress = core.snapshot().motion.as_ref().unwrap().progress;
                character
                    .tick(now + 0.02, &mut core, false, None, false, None, None)
                    .unwrap();
                assert_eq!(character.status.state, CharacterState::Settling);
                assert_eq!(core.snapshot().motion.as_ref().unwrap().progress, progress);
                assert_eq!(character.speech_motion_run_id, run);
            }
        }
    }

    #[test]
    fn playback_completion_preserves_measured_settling_of_final_plan() {
        let mut core = following_core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.active_anchor = Some(core.poses().pose("home").unwrap().clone());
        character.note_speech_started(0.0);
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 100],
            quiet_regions: vec![],
            phrase_peaks: vec![],
            duration_seconds: 2.0,
            streaming: false,
        };
        character
            .tick(0.0, &mut core, false, None, true, Some(&analysis), Some(0))
            .unwrap();
        let run = character.speech_motion_run_id;
        for frame in 1..500 {
            let now = frame as f64 * 0.02;
            core.tick(now).unwrap();
            if core.snapshot().motion.as_ref().unwrap().state == MovementPhase::Settling {
                character
                    .tick(now, &mut core, false, None, false, None, None)
                    .unwrap();
                assert_eq!(character.speech_motion_run_id, run);
                assert_eq!(
                    core.snapshot().motion.as_ref().unwrap().state,
                    MovementPhase::Settling
                );
                assert_eq!(character.status.state, CharacterState::Settling);
                return;
            }
            character
                .tick(
                    now,
                    &mut core,
                    false,
                    None,
                    true,
                    Some(&analysis),
                    Some(frame),
                )
                .unwrap();
        }
        panic!("final plan never reached measured settling");
    }

    #[test]
    fn ending_speech_interrupts_the_long_performance_and_blends_to_anchor() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 500],
            quiet_regions: vec![],
            phrase_peaks: vec![80, 240, 400],
            duration_seconds: 10.0,
            streaming: false,
        };
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.note_speech_started(0.0);

        character
            .tick(0.1, &mut core, false, None, true, Some(&analysis), Some(0))
            .unwrap();
        let performance_run = character.speech_motion_run_id.unwrap();
        assert_eq!(core.mode(), RuntimeMode::Moving);

        character
            .tick(0.2, &mut core, false, None, false, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::Settling);
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("speak_settle")
        );
        assert_eq!(character.speech_motion_run_id, Some(performance_run));

        let mut now = 0.2;
        while character.status.state == CharacterState::Settling {
            now += 0.1;
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
            assert!(now < 10.0, "speech settle did not terminate");
        }
        assert_eq!(character.status.state, CharacterState::HomeIdle);
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        assert!(character.speech_motion_run_id.is_none());
    }

    #[test]
    fn neutral_playback_acknowledgement_preserves_settle_and_next_speech() {
        let mut core = following_core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 500],
            quiet_regions: vec![],
            phrase_peaks: vec![80, 240, 400],
            duration_seconds: 10.0,
            streaming: false,
        };
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.note_speech_started(0.0);
        character
            .tick(0.1, &mut core, false, None, true, Some(&analysis), Some(0))
            .unwrap();
        for reaction in ["neutral", "listening", "thinking"] {
            character.set_reaction(reaction, 0.15, &mut core).unwrap();
            assert_eq!(character.status.state, CharacterState::Speaking);
        }
        core.tick(0.2).unwrap();
        character
            .tick(0.2, &mut core, false, None, false, None, None)
            .unwrap();
        let settle_run = character.speech_motion_run_id.unwrap();

        // Studio sees audio completion before physical settling has completed.
        character.set_reaction("neutral", 0.21, &mut core).unwrap();
        assert_eq!(character.status.state, CharacterState::Settling);
        advance(&mut character, &mut core, 0.21, 4.21);
        assert!(character.speech_motion_run_id.is_none());
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));

        // A later idle replaces the runtime's most recent terminal movement.
        character.next_micro_at = 4.21;
        advance(&mut character, &mut core, 4.21, 12.21);
        assert_ne!(
            core.snapshot().last_motion.as_ref().unwrap().run_id,
            settle_run
        );
        character
            .preempt_idle_or_thinking(12.22, &mut core)
            .unwrap();
        character.note_speech_started(12.22);
        character
            .tick(
                12.24,
                &mut core,
                false,
                None,
                true,
                Some(&analysis),
                Some(0),
            )
            .unwrap();
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("speaking_performance")
        );
        assert_eq!(core.mode(), RuntimeMode::Moving);
    }

    #[test]
    fn speech_recovers_when_prior_movement_left_terminal_history() {
        let mut core = following_core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let prior = core
            .play_anchored_relative("idle_breathe", anchor.clone(), 0.0)
            .unwrap();
        for step in 1..=500 {
            core.tick(step as f64 * 0.02).unwrap();
        }
        core.play_anchored_relative("idle_breathe", anchor.clone(), 10.0)
            .unwrap();
        for step in 501..=1000 {
            core.tick(step as f64 * 0.02).unwrap();
        }
        assert!(terminal_phase(&core, prior).is_none());
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.active_anchor = Some(anchor);
        character.speech_motion_run_id = Some(prior);
        character.note_speech_started(20.0);
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 500],
            quiet_regions: vec![],
            phrase_peaks: vec![80, 240, 400],
            duration_seconds: 10.0,
            streaming: false,
        };
        character
            .tick(
                20.02,
                &mut core,
                false,
                None,
                true,
                Some(&analysis),
                Some(0),
            )
            .unwrap();
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("speaking_performance")
        );
        assert_eq!(core.mode(), RuntimeMode::Moving);
    }

    #[test]
    fn generated_speech_performance_remains_readable_at_every_supported_anchor() {
        let core = core();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let calibration = crate::motion::calibration::load_calibration_file(
            root.join("simulation/mujoco/config/servo_calibration.json"),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        let limits: Vec<JointLimit> = calibration
            .iter()
            .map(|joint| {
                let (lower_rad, upper_rad) = joint.safe_range_radians();
                JointLimit {
                    name: joint.name.clone(),
                    lower_rad,
                    upper_rad,
                }
            })
            .collect();
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 1_000],
            quiet_regions: vec![],
            phrase_peaks: vec![100, 300, 500, 700, 900],
            duration_seconds: 20.0,
            streaming: false,
        };
        for anchor_name in ["home", "attentive", "look_left", "look_right"] {
            let anchor = core.poses().pose(anchor_name).unwrap().clone();
            let mut character = CharacterCoordinator::new(42);
            let performance = character
                .compose_speech_performance(&analysis, core.motions(), &anchor)
                .unwrap();
            let scale = performance
                .uniform_amplitude_scale(&anchor, &limits)
                .unwrap();
            assert!(scale > 0.75, "{anchor_name} collapsed to scale {scale}");
            let zero_velocity = anchor.keys().map(|joint| (joint.clone(), 0.0)).collect();
            let sequence = MotionSequence::compile_scaled_calibrated(
                &performance,
                anchor.clone(),
                zero_velocity,
                anchor.clone(),
                scale,
                &limits,
            )
            .unwrap();
            for sample in 0..=100 {
                let positions = sequence
                    .sample(sequence.duration_seconds() * sample as f64 / 100.0)
                    .unwrap();
                for limit in &limits {
                    assert!((limit.lower_rad..=limit.upper_rad).contains(&positions[&limit.name]));
                }
            }
            assert_eq!(
                sequence.sample(sequence.duration_seconds()).unwrap(),
                anchor
            );
        }
    }

    #[test]
    fn only_a_completed_scene_replaces_the_idle_anchor() {
        let core = &mut core();
        let home = core.poses().pose("home").unwrap().clone();
        let measured: JointPositions = core
            .snapshot()
            .joints
            .iter()
            .map(|joint| (joint.name.clone(), joint.position))
            .collect();
        assert_ne!(home, measured);

        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(home.clone());
        character.note_foreground_scene_started(1.0, 7);
        character
            .tick(1.1, core, false, Some((7, false)), false, None, None)
            .unwrap();
        assert_eq!(character.status.active_anchor.as_ref(), Some(&home));

        character.note_foreground_scene_started(2.0, 8);
        character
            .tick(2.1, core, false, Some((8, true)), false, None, None)
            .unwrap();
        assert_eq!(character.status.active_anchor.as_ref(), Some(&measured));
    }

    #[test]
    fn idle_selection_never_immediately_repeats() {
        let core = core();
        let mut character = CharacterCoordinator::new(17);
        character.status.active_anchor = Some(core.poses().pose("home").unwrap().clone());
        for category in [NextIdleCategory::Micro, NextIdleCategory::Large] {
            let mut previous = None;
            for _ in 0..50 {
                character.last_idle = previous.clone();
                let selected = character.choose_idle(category, &core);
                assert_ne!(Some(selected.clone()), previous);
                previous = Some(selected);
            }
        }
    }

    #[test]
    fn directional_idles_avoid_yaw_clips_that_collapse_at_the_right_limit() {
        let core = core();
        let mut character = CharacterCoordinator::new(17);
        character.status.active_anchor = Some(core.poses().pose("look_right").unwrap().clone());

        for category in [NextIdleCategory::Micro, NextIdleCategory::Large] {
            for _ in 0..100 {
                let selected = character.choose_idle(category, &core);
                assert!(!matches!(
                    selected.as_str(),
                    "idle_micro_glance" | "idle_soft_head_shake"
                ));
                character.last_idle = Some(selected);
            }
        }
    }

    #[test]
    fn speech_selection_is_seeded_weighted_and_never_immediately_repeats() {
        let mut first = CharacterCoordinator::new(91);
        let mut second = CharacterCoordinator::new(91);
        let mut calm = 0;
        let mut explanatory = 0;
        let mut first_sequence = Vec::new();
        let mut second_sequence = Vec::new();

        for _ in 0..1_000 {
            first.last_speech_clip = None;
            let clip = first.choose_speech_clip(false);
            if clip == "speak_calm_sway" {
                calm += 1;
            } else if clip == "speak_explanatory_lean" {
                explanatory += 1;
            }
            first_sequence.push(clip);

            second.last_speech_clip = None;
            let clip = second.choose_speech_clip(false);
            second_sequence.push(clip);
        }

        assert_eq!(first_sequence, second_sequence);
        assert!(calm > explanatory * 2);

        for emphasis in [false, true] {
            let mut character = CharacterCoordinator::new(17);
            let mut previous = None;
            for _ in 0..100 {
                character.last_speech_clip = previous.clone();
                let selected = character.choose_speech_clip(emphasis);
                assert_ne!(Some(selected.clone()), previous);
                previous = Some(selected);
            }
        }
    }

    #[test]
    fn mechanical_rest_turns_character_off_without_scheduling_animation() {
        let mut core = core();
        let mut character = CharacterCoordinator::new(51);
        character.status.enabled = true;
        character.status.state = CharacterState::PoseIdle;
        character.status.active_anchor = Some(core.poses().pose("rest").unwrap().clone());
        character.status.next_idle_category = Some(NextIdleCategory::Micro);

        character
            .tick(100.0, &mut core, false, None, false, None, None)
            .unwrap();

        assert!(!character.status.enabled);
        assert_eq!(character.status.state, CharacterState::Off);
        assert!(character.status.active_anchor.is_none());
        assert!(character.status.active_clip.is_none());
        assert!(character.status.next_idle_category.is_none());
    }
}
