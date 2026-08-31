# Orion Guidebook

## From a LeLamp-Based Prototype to an Expressive Robotic Lamp

**Project:** Orion  
**Document type:** High-level development roadmap  
**Current position:** Native Rust runtime validated in MuJoCo and on hardware; device integration next
**Long-term goal:** Build a safe, useful, expressive robotic lamp inspired by LeLamp, Watti, Ongo, and the ELEGNT movement-design framework.

---

# 1. Purpose of This Guidebook

Orion is a long-term robotics project to create an articulated robotic lamp that can:

- Function as a genuinely useful desk lamp.
- Move safely and smoothly.
- Direct its light toward people, objects, and work areas.
- Communicate attention, intention, attitude, and apparent emotion through movement.
- Perceive its surrounding workspace.
- Respond through coordinated motion, light, sound, and voice.
- Execute behaviours locally and predictably.
- Eventually use AI for higher-level interaction without allowing AI to bypass safety-critical control.
- Progress from a LeLamp-compatible reference platform to original Orion hardware.

This guidebook describes the major milestones required to reach that goal.

It is **not** an implementation manual. Each milestone should eventually have its own focused design document containing:

- System context.
- Concepts to learn.
- Architecture.
- Implementation steps.
- Validation procedures.
- Failure modes.
- Lessons learned.

The purpose of this guidebook is to prevent Orion from becoming a collection of disconnected experiments. Every new feature should fit into a deliberate sequence.

> **Architecture decision — 2026-08-29:** Orion uses the native Rust `oriond`
> runtime for physical control and MuJoCo, with `rustypot` providing the
> STS3215 protocol and serial boundary. This Rust implementation is Orion's
> sole active runtime and has been confirmed in software, MuJoCo, and on the
> physical robot. ROS 2 is no longer part of the implementation. Later
> ROS-specific milestone text is retained as historical planning context and
> is superseded by `docs/orion_control_architecture.md`.

---

# 2. What Orion Is Ultimately Trying to Become

Orion should eventually feel like more than a motorised lamp, while still remaining useful as a lamp.

The desired experience is:

> Orion notices what the user is doing, understands where their attention is directed, positions its light helpfully, communicates its own intent through movement, and responds in a way that feels deliberate rather than mechanical.

A mature Orion should be able to perform interactions such as:

- Wake up and acknowledge the user.
- Turn toward the person speaking.
- Follow a user’s hand or active work area.
- Illuminate a book, keyboard, electronic component, or object.
- Nod or otherwise acknowledge an instruction.
- Communicate that a requested target is unreachable.
- Use subtle movement while listening.
- Use light for timers, notifications, and system states.
- Coordinate movement, light, and voice.
- Perform expressive social behaviours without compromising its primary function.
- Return to a safe resting pose when idle.
- Continue operating safely when perception, networking, or AI services fail.

Orion must not become:

- An LLM directly controlling five motors.
- A collection of hard-coded demos with no reusable architecture.
- A visually expressive robot that is poor at being a lamp.
- A simulator-only project that ignores physical constraints.
- A hardware build whose software is inseparable from one servo model.
- A robot that moves constantly without a reason.
- A system that communicates capabilities it does not actually possess.

---

# 3. Reference Projects and What Orion Should Learn from Them

## 3.1 LeLamp: the reference embodiment

LeLamp provides Orion with a practical starting point:

- Five-axis articulated movement.
- A buildable mechanical design.
- Camera, microphone, speaker, and programmable-light concepts.
- Record-and-replay movement.
- Public 3D and simulation assets.
- A comparatively accessible route to physical hardware.

Orion should use LeLamp to learn:

- Mechanical assembly.
- Servo configuration.
- Joint calibration.
- Cable routing.
- Power distribution.
- The relationship between simulated and physical joints.
- What five degrees of freedom can and cannot express.

LeLamp is Orion’s **reference platform**, not Orion’s permanent identity.

The progression should be:

> Reproduce LeLamp’s embodiment → understand it → develop independent control software → modify it → collect evidence → design custom Orion hardware.

Because Orion currently contains assets derived from a GPL-3.0-licensed repository, the project should maintain clear source provenance and review the licensing implications before distributing derivative assets or software.

---

## 3.2 Watti: scene authoring and local execution

Watti demonstrates several architectural ideas that are highly relevant to Orion:

- Five motorised axes.
- An RGB-D camera.
- Addressable LEDs.
- A browser-based editor.
- A timeline combining poses and lighting.
- Inverse kinematics and target-looking tools.
- Virtual scene preview.
- Whole-scene transfer to the lamp.
- Local playback rather than frame-by-frame browser control.

A particularly valuable Watti principle is:

> The editor describes the scene, but the lamp owns timing, limits, playback, and emergency stopping.

Watti packages the complete motion-and-light scene, sends it to the lamp, and plays it locally on a shared timeline. The browser does not need to remain connected for every frame.

Orion should eventually adopt the same separation:

```text
Orion Studio
    describes and previews a scene
                |
                v
Orion runtime
    validates, stores, and executes it locally
                |
                v
Safety-controlled hardware
```

The Orion Studio interface should never be the component responsible for enforcing motor limits or emergency stops.

---

## 3.3 Ongo: the product-experience target

Ongo publicly presents itself as a “living lamp” built around movement, listening, ambient presence, and contextual awareness. Its public materials are more useful as a reference for the desired product experience than as a disclosed engineering architecture.

Orion should learn from that product ambition:

- The robot should fit naturally into a room.
- It should not demand constant attention.
- Idle behaviour matters.
- Movement should feel intentional.
- The lamp should remain useful even when advanced intelligence is disabled.
- The interaction should feel ambient rather than like operating industrial machinery.
- The enclosure, noise level, cable management, and quality of light matter as much as the AI demonstration.

---

## 3.4 ELEGNT: the movement-design framework

ELEGNT provides Orion with its central interaction-design principle:

> A robot should not only move to complete a task. Its movement should also communicate intention, attention, attitude, and apparent emotion.

The paper separates:

- **Functional utility:** whether the robot completes the physical task.
- **Expressive utility:** what the movement communicates to the user.

Conceptually:

\[
\text{Movement objective} = F(\tau) + \gamma E(\tau)
\]

where:

- \(F(\tau)\) measures functional success.
- \(E(\tau)\) represents expressive value.
- \(\gamma\) controls how much expression is appropriate for the context.

The paper found that expressive movements improved engagement and perceived robot qualities, particularly in social interactions. It also found that unnecessary expression could interfere with function-oriented tasks, meaning expressiveness must be contextual rather than constant.

ELEGNT should therefore influence the entire Orion project. It is not a feature that gets added near the end.

---

# 4. Orion’s Core Mental Model

Every Orion action should be understood as several separate layers.

## 4.1 Functional goal

The functional controller decides:

> What physical outcome must be achieved?

Examples:

- Point the light at a book.
- Turn toward the speaker.
- Return to the resting pose.
- Track the active work area.
- Move the lamp head away from an obstruction.

---

## 4.2 Expressive intent

The expressive layer decides:

> What should the user infer from the movement?

Examples:

- Orion has noticed the book.
- Orion is paying attention to the user.
- Orion understood the instruction.
- Orion is uncertain.
- Orion cannot reach the target.
- Orion is calm, curious, excited, or tired.

---

## 4.3 Motion generation

The motion system decides:

> What safe trajectory should achieve the functional goal while expressing the intended state?

It controls:

- Joint positions.
- Motion duration.
- Speed.
- Acceleration.
- Deceleration.
- Pauses.
- Anticipatory movements.
- Head tilts.
- Leaning.
- Overshoot and settling.
- Coordination between joints.

---

## 4.4 Safety validation

The safety layer decides:

> Is this movement allowed to execute?

It must enforce:

1. Joint limits.
2. Velocity limits.
3. Acceleration and jerk limits.
4. Self-collision restrictions.
5. Base-stability constraints.
6. Motor effort and temperature limits.
7. Communication timeouts.
8. Emergency-stop state.
9. Forbidden workspace regions.
10. Human-proximity restrictions.

---

## 4.5 Execution backend

The backend decides where the validated movement is executed:

```text
MuJoCo
Physical STS3215 servos
Future custom Orion actuators
```

The higher-level motion and behaviour systems should not need to know which backend is active.

---

# 5. Priority Hierarchy

Orion should use this hierarchy whenever objectives conflict:

```text
1. Human and hardware safety
2. Functional correctness
3. Predictability and legibility
4. Expressiveness
5. Efficiency and visual style
```

An expressive trajectory is unsuccessful when it:

- Tips the lamp over.
- Misses the lighting target.
- Confuses the user about Orion’s intent.
- Exceeds motor limits.
- Creates a pinch hazard.
- Causes unnecessary noise or vibration.
- Takes so long that it interferes with the task.

Expression must operate inside a safe and functionally valid envelope.

---

# 6. Development Principles

## 6.1 Simulation-first, not simulation-only

MuJoCo should be used to:

- Validate kinematics.
- Test control interfaces.
- Prototype motion.
- Evaluate stability.
- Check collisions.
- Test target pointing.
- Develop perception.
- Measure trajectories.

However, simulation cannot fully represent:

- Servo backlash.
- Gear noise.
- Print flex.
- Cable drag.
- Heat.
- Electrical current spikes.
- Brownouts.
- Connector failures.
- Real lighting quality.
- Physical pinch points.

Every major subsystem should move through:

```text
Software model
    → simulation validation
        → small physical bench test
            → complete physical validation
```

---

## 6.2 Build one layer at a time

Do not introduce:

- New hardware.
- New perception.
- New behaviour logic.
- New AI services.
- New motion formats.

all in the same milestone.

Each milestone should produce a known-working baseline.

---

## 6.3 Maintain simulator-independent semantics

Orion should use the same:

- Joint names.
- Pose names.
- Motion files.
- Coordinate conventions.
- Target-point representations.
- Behaviour names.

across MuJoCo and physical hardware.

---

## 6.4 Keep deterministic control below AI

An LLM may eventually select:

- A recognised behaviour.
- A target.
- An expression profile.
- A lighting scene.
- A spoken response.

It must not directly generate unrestricted:

- Motor positions.
- Servo register values.
- Velocities.
- Torques.
- Emergency-stop decisions.

AI belongs above Orion’s validated capability interface.

---

## 6.5 Co-design movement and hardware

ELEGNT argues that movement, form, and interaction scenarios should evolve together. A custom Orion body must therefore be designed around the movements it needs to perform, not only around appearance.

Questions such as these should guide future hardware design:

- Can the head tilt far enough to communicate curiosity?
- Can Orion nod without colliding with its arm?
- Can it move slowly without jitter?
- Can it move quickly without tipping?
- Does its camera placement make its gaze understandable?
- Does the light point in the direction users interpret as Orion’s attention?
- Is the base stable enough for expressive acceleration?
- Are the actuators quiet enough for a calm domestic environment?

---

# 7. Target System Architecture

```text
                         Human and environment
                                  |
                     camera / depth / audio / touch
                                  |
                                  v
                         Orion perception
                    people / hands / objects /
                     speech / workspace state
                                  |
                                  v
                         Orion world model
               targets, confidence, attention, context
                                  |
                                  v
                        Behaviour orchestrator
                 task state, interaction state, policy
                         /                    \
                        /                      \
                       v                        v
             Functional goal             Expression profile
          illuminate target, look,     attention, intention,
          return home, acknowledge     attitude, apparent emotion
                         \                    /
                          \                  /
                           v                v
                         Motion composer
               poses, keyframes, IK, interpolation
                                  |
                                  v
                        Motion safety layer
                  limits, collision, stability,
                    timeouts, effort, emergency stop
                                  |
                                  v
                    Native motion interface
                                  |
                       ┌───────────┴───────────┐
                       v                       v
                    MuJoCo              Physical Orion
                native adapter        Rust STS3215 runtime
              
              ┌───────────────────┴───────────────────┐
              v                                       v
       Light and LED runtime                    Audio runtime
```

A later Orion Studio application should sit outside the safety-critical runtime:

```text
Orion Studio
    |
    | creates and previews scenes
    v
Scene validation and transfer
    |
    v
Local Orion scene player
```

---

# 8. Milestone Roadmap

| Milestone | Name | Current status |
|---:|---|---|
| 0 | Product and interaction charter | **In progress** — charter drafted |
| 1 | Robot description and simulation foundation | **Complete** |
| 2 | Motion foundation and first expressive behaviours | **Complete** |
| 3 | Motion quality, safety, and simulator parity | **Complete** |
| 4 | Task-space control and target pointing | **Next** |
| 5 | Lighting and multimodal scene runtime | Planned |
| 6 | Orion Studio motion-and-light editor | Planned |
| 7 | Physical LeLamp-compatible prototype | **In progress** — calibrated motion validated |
| 8 | Native hardware runtime and sim-to-real transfer | **In progress** — Rust runtime confirmed in MuJoCo and on hardware |
| 9 | Perception and world model | Planned |
| 10 | Attention and adaptive task lighting | Planned |
| 11 | Behaviour orchestration | Planned |
| 12 | Voice and agent integration | **In progress** — local TTS and wake-word detection implemented; speech-to-text, intent routing, and agent integration remain |
| 13 | Context-aware ELEGNT expression engine | Planned |
| 14 | Custom Orion mechanical and electrical design | Planned |
| 15 | Product hardening, evaluation, and release | Planned |
| 16 | Optional advanced capabilities | Future |

---

# 9. Milestone 0 — Product and Interaction Charter

## Status

**In progress.** The first product-owner interview was recorded in
`docs/product/orion_product_charter.md` on 2026-08-22. The main product
direction is defined, but the scenario, character-guide, sensor-mapping, and
out-of-scope work required for closeout is not complete.

## Confirmed decisions

- Orion is an expressive companion robot for screen-free embodied AI
  interaction.
- Its first audience is a broad household audience.
- It is a stationary tabletop robot that may be carried between suitable
  surfaces.
- It has a desk-lamp-sized base and a larger arm working area across the desk.
- Ambient room lighting is the priority, supported by task and expressive
  lighting.
- Its character is curious, calm, playful, mature, and unintrusive.
- Speech begins after a user addresses Orion.
- Limited proactive greeting, lighting, reminder, notification, music, and
  household-activity responses are allowed when they remain unintrusive.
- Core operation, perception, conversation, and approved memory must work
  locally.
- Raw camera and microphone data is processed locally and discarded. Only
  selected preferences, reminders, and user-approved memories may be retained.
- Household data must not leave the device.

## Remaining closeout work

- Agree the complete first-version out-of-scope list.
- Define safe movement boundaries around people and household objects.
- Complete the six required interaction storyboards.
- Create functional, expressive, reactive, and proactive variants for each
  scenario.
- Define proactive frequency, interruption, quiet-hour, and notification rules.
- Create `docs/personality/orion_character_guide.md`.
- Connect every proposed sensor and feature to an approved scenario.
- Review the finished artifacts against the Milestone 0 exit criteria.

## Objective

Define what Orion should do before deciding how to implement it.

## Work required

Create a short product charter covering:

- Orion’s primary user.
- Where it will operate.
- The size of its workspace.
- Its core lighting function.
- Its desired personality.
- What proactive behaviour is acceptable.
- What data it may collect.
- Which operations must work locally.
- Which behaviours are explicitly out of scope.

Create interaction storyboards for at least six scenarios:

1. Task-light positioning.
2. User acknowledgement.
3. Following a hand or work area.
4. Timer or quiet notification.
5. Unreachable-target failure.
6. Social conversation or music interaction.

For each scenario, create:

- A purely functional version.
- An expressive version.
- A reactive version, where the user initiates the interaction.
- A proactive version, where Orion initiates the interaction where appropriate.

ELEGNT used storyboarding and research-through-design to explore how functional, social, proactive, and reactive interactions should differ.

## Deliverables

```text
docs/product/orion_product_charter.md
docs/scenarios/
docs/personality/orion_character_guide.md
```

## Exit criteria

- Orion’s initial use cases are clearly defined.
- The project can explain what Orion will not attempt in its first version.
- Every proposed sensor or feature is connected to a real interaction scenario.

---

# 10. Milestone 1 — Robot Description and Simulation Foundation

## Status

**Complete.**

## Completed work

Orion now has:

- A backend-neutral robot description and shared mesh library.
- Five semantically named joints.
- A ground-contact `base_footprint`.
- A native MuJoCo model.
- Matching joint and actuator names across description and simulation.
- Free-standing physics.
- Joint feedback.
- Valid trajectory execution.
- Learning documentation.

## Why this milestone matters

This establishes a reliable digital body for Orion before the project develops a brain.

The neutral URDF validates:

- Geometry.
- Joint relationships.
- Axes.
- Limits.

MuJoCo validates:

- Native actuator dynamics.
- Stability.
- Tipping.
- More aggressive dynamic motion.

## Exit criteria

Already satisfied:

- All five joints behave correctly.
- Meshes load.
- Commands execute.
- The native runtime builds and MuJoCo model loads.

---

# 11. Milestone 2 — Motion Foundation and First ELEGNT Behaviours

## Objective

Build Orion’s first original subsystem:

> A simulator-independent motion package capable of executing named poses and timed keyframe animations.

Do not add:

- Voice.
- Cameras.
- LLMs.
- Autonomous behaviour.
- A full ELEGNT optimiser.
- Custom physical hardware.

The purpose of this milestone is to understand how motion is represented, validated, and executed.

---

## Core mental model

A **pose** describes where Orion should be:

```text
home
neutral
rest
attentive
look_left
look_right
curious_left
curious_right
```

A **trajectory** describes how Orion moves through time:

```text
wake_up
nod
shake_no
inspect_target
acknowledge
unreachable_target
```

Two movements may finish at exactly the same pose while communicating completely different attitudes because their:

- Timing.
- Pauses.
- Speed.
- Acceleration.
- Joint coordination.

are different.

---

## Source directory

```text
motion/
```

Suggested initial structure:

```text
orion_motion/
├── config/
│   ├── poses.yaml
│   └── motion_limits.yaml
├── motions/
│   ├── functional/
│   └── expressive/
├── orion_motion/
│   ├── motion_loader.py
│   ├── motion_player.py
│   ├── trajectory_builder.py
│   └── motion_validator.py
├── launch/
├── test/
├── package.xml
└── setup.py
```

---

## First three functional and expressive pairs

### 1. Look at a target

**Functional version**

```text
Turn directly toward the target.
```

**Expressive version**

```text
Brief anticipatory glance
    → slight head tilt
        → lean toward the target
            → move to final pose
                → settle
```

### 2. Acknowledge the user

**Functional version**

```text
No physical movement is required.
```

**Expressive version**

```text
Orient toward the user
    → small nod
        → return to attentive posture
```

### 3. Target unreachable

**Functional version**

```text
Stop and return a failure result.
```

**Expressive version**

```text
Pause
    → move safely toward the reach boundary
        → return attention to the user
            → small head shake
                → settle safely
```

At this stage, “look at target” may use a predefined joint-space target. Actual 3D target pointing belongs to Milestone 4.

---

## Deliverables

```text
backend-independent motion library
named-pose library
keyframe motion format
native Rust motion player
joint-limit validation
MuJoCo playback using the same motion files
three functional/expressive A/B pairs
unit tests
learning documentation
```

## Exit criteria

- A named pose can be requested by name.
- A multi-keyframe animation executes on the native Rust runtime.
- The same animation definition executes in MuJoCo.
- Invalid joint values are rejected.
- All five joints are represented consistently.
- The three functional/expressive pairs are visibly different.
- No perception or AI dependency is required.

---

# 12. Milestone 3 — Motion Quality, Safety, and Simulator Parity

## Objective

Turn basic keyframe playback into a dependable motion system.

## Work required

Add:

- Smooth interpolation.
- Start-state continuity.
- Velocity limits.
- Acceleration limits.
- Jerk-aware transitions.
- Motion cancellation.
- Preemption.
- Feedback and result reporting.
- Timeout handling.
- Safe return-to-rest behaviour.
- Base-stability checks.
- Known forbidden poses.
- Automated trajectory validation.

The system should distinguish:

```text
requested keyframes
generated trajectory
validated trajectory
executed trajectory
measured trajectory
```

These are not always identical.

## Cross-backend validation

The same motion file should produce meaningfully equivalent movement in:

- MuJoCo.
- Physical hardware.

The simulators do not need to have identical physics, but they must agree on:

- Joint semantics.
- Timing.
- Target positions.
- Joint limits.
- Direction conventions.
- Motion names.

## Deliverables

```text
interpolation library
motion preemption
motion cancellation
trajectory feedback
stability checks
cross-simulator playback tests
motion validation report
```

## Exit criteria

- Motions begin from the measured current position.
- No position discontinuity occurs at startup.
- Motions can be stopped safely.
- Unsafe or malformed motions are rejected.
- Named motions remain portable between MuJoCo and physical hardware.
- Aggressive movements that risk tipping are detected or explicitly marked unsafe.

---

# 13. Milestone 4 — Task-Space Control and Target Pointing

## Objective

Move from:

> Set these five joint angles.

to:

> Point Orion’s light at this position in space.

## Work required

Add and validate frames such as:

```text
lamp_head_link
light_axis_frame
gaze_frame
camera_link
camera_optical_frame
```

Learn and implement:

- Forward kinematics.
- Transform trees.
- Coordinate-frame conversion.
- Workspace representation.
- Pointing constraints.
- Numerical inverse kinematics.
- Preferred postures.
- Joint-limit handling.
- Unreachable-target detection.
- Collision-aware target rejection.

The initial problem should be constrained:

> Align `light_axis_frame` with a target point while selecting a safe and comfortable posture.

Do not initially demand an exact six-dimensional head pose.

## Deliverables

```text
orion_kinematics package
forward-kinematics tools
target-point interface
look-at solver
workspace visualisation
unreachable-target result
pointing-error measurements
```

## Exit criteria

- A target point can be published in a known frame.
- Orion points its light axis toward the target.
- Joint limits remain respected.
- Unreachable targets return an explicit failure.
- Functional and expressive target-pointing variants can reach the same target.

---

# 14. Milestone 5 — Lighting and Multimodal Scene Runtime

## Objective

Treat light as part of Orion’s behaviour rather than a separate accessory.

Orion will eventually need two lighting concepts:

1. **Task illumination**
   - White-light brightness.
   - Colour temperature.
   - Beam direction.
   - Useful desk illumination.

2. **Expressive illumination**
   - RGB colour.
   - Pulsing.
   - Fades.
   - Status patterns.
   - Emotional or behavioural accents.

## Work required

Create a scene format that can coordinate:

```text
joint poses
joint transitions
task-light brightness
RGB-light state
sound events
looping
pauses
behaviour metadata
```

The scene should execute locally on Orion using one authoritative clock.

Following Watti’s architecture, the editor or remote application should transfer a complete scene rather than continuously steering every animation frame over the network.

## Deliverables

```text
orion_lighting package
scene schema
local scene player
motion-and-light synchronisation
scene validator
example expressive scenes
```

## Exit criteria

- Motion and light remain synchronised.
- Playback continues if the editor disconnects.
- Lighting failures do not bypass motion safety.
- Functional task lighting remains independently controllable.
- Scene timing is deterministic.

---

# 15. Milestone 6 — Orion Studio

## Objective

Create a visual tool for authoring, previewing, validating, and transferring Orion scenes.

## Initial Studio capabilities

- Display a 3D Orion model.
- Move individual joints.
- Save named poses.
- Place poses on a timeline.
- Edit transition durations.
- Edit lighting states.
- Preview scenes virtually.
- Validate limits.
- Export a simulator-independent scene file.
- Send a complete scene to the Orion runtime.

## Later Studio capabilities

- Curve editing.
- Motion easing controls.
- Expression profiles.
- Target-point markers.
- Inverse-kinematics tools.
- Camera and depth preview.
- Live physical-pose feedback.
- Scene comparison.
- Functional/expressive A/B playback.
- Stability warnings.
- Collision warnings.

## Architectural boundary

```text
Studio:
authoring, preview, transfer

Orion runtime:
validation, timing, limits, execution, emergency stop
```

## Deliverables

```text
orion_studio/
3D model viewer
pose editor
scene timeline
virtual preview
scene export and transfer
```

## Exit criteria

- A scene can be created without editing YAML manually.
- The same exported scene works in MuJoCo and physical hardware.
- The runtime remains responsible for final validation.
- Closing the browser does not interrupt local playback.

---

# 16. Milestone 7 — Physical LeLamp-Compatible Prototype

## Objective

Build a known physical reference platform before designing custom Orion mechanics.

## Development sequence

### Step 1: single-servo bench test

Test one STS3215 servo and controller:

- Assign servo ID.
- Command position.
- Read position.
- Enable and disable torque.
- Apply limits.
- Measure current.
- Monitor temperature.
- Test communication loss.
- Test emergency stopping.

### Step 2: one loaded joint

Build one representative joint with a dummy load.

Evaluate:

- Holding torque.
- Heat.
- Backlash.
- Noise.
- Mechanical flex.
- Power behaviour.
- Cable movement.

### Step 3: complete reference lamp

Assemble the LeLamp-compatible mechanism with minimal modification.

Validate:

- Joint directions.
- Zero positions.
- Mechanical limits.
- Cable routing.
- Base stability.
- Emergency stop.
- Manual movement.
- Safe low-speed trajectories.

## Deliverables

```text
physical five-axis lamp
power-distribution diagram
emergency-stop mechanism
servo inventory and ID map
joint calibration sheet
assembly notes
hardware safety checklist
```

## Exit criteria

- All five physical joints can be moved safely.
- Orion has a reliable emergency stop.
- Every joint has measured zero, direction, and limits.
- The lamp can hold neutral and resting poses.
- Basic movement works before cameras, voice, or AI are added.

---

# 17. Milestone 8 — Native Hardware Runtime and Sim-to-Real Transfer

## Objective

Make the physical lamp and MuJoCo consume the same motion semantics through
native backend adapters.

## Work required

The implemented STS3215 Rust runtime converts between:

```text
Orion joint radians
```

and:

```text
servo encoder positions
```

The interface must manage:

- Servo IDs.
- Direction signs.
- Zero offsets.
- Encoder conversion.
- Position commands.
- Position feedback.
- Communication timeouts.
- Torque enable and disable.
- Error states.
- Temperature and current telemetry where available.
- Safe startup.
- Safe shutdown.

## Core architecture

```text
shared motion YAML
    |
native motion library
    |
oriond 50 Hz control loop
    |
STS3215 driver and transport
    |
physical servos
```

High-level motion software should not contain direct servo-register operations.

## Deliverables

```text
native Rust runtime
joint calibration configuration
local command/status socket
watchdog
hardware diagnostics
shared Rust simulation/hardware backend selector
```

## Exit criteria

- The same pose and motion definitions control MuJoCo and the physical lamp.
- A named motion can run in simulation and hardware without rewriting it.
- Calibration offsets are configuration, not hard-coded behaviour logic.
- Communication failure disables or safely stops motion.
- Physical motion remains within measured limits.

---

# 18. Milestone 9 — Perception and World Model

## Objective

Give Orion a grounded representation of people, objects, and the workspace.

## Development order

### Stage 1: RGB perception

Start with:

- Person detection.
- Face detection.
- Hand tracking.
- Basic object detection.
- User-presence detection.

### Stage 2: camera calibration

Establish:

- Camera intrinsics.
- Camera-to-head transform.
- Camera optical frame.
- Relationship between gaze direction and physical camera direction.

### Stage 3: depth perception

Add RGB-D capabilities for:

- 3D target localisation.
- Desk-plane detection.
- Hand position in 3D.
- Distance measurement.
- Object geometry.
- Workspace scanning.

### Stage 4: world model

Represent:

- Current user.
- User position.
- Active hand.
- Active object.
- Work surface.
- Target confidence.
- Last-seen time.
- Occlusion state.
- Reachability.
- Current attention target.

## Important boundary

Perception should publish observations and confidence.

It should not directly command motors.

```text
Perception:
“I believe the active hand is at this 3D point with 82% confidence.”

Behaviour and motion:
“Given that observation, should Orion move?”
```

## Deliverables

```text
orion_perception package
camera calibration
person and hand tracking
desk-plane estimation
3D target output
world-state representation
confidence and timeout handling
```

## Exit criteria

- A target can be located consistently in Orion’s base frame.
- Lost targets are explicitly reported.
- Stale perception data cannot command movement.
- Orion’s physical gaze is consistent with its camera placement.
- Privacy and data-retention choices are documented.

---

# 19. Milestone 10 — Attention and Adaptive Task Lighting

## Objective

Combine perception with task-space control.

This is the milestone where Orion begins to behave like an intelligent lamp rather than an animated robot arm.

## Initial behaviours

- Turn toward the current user.
- Track a hand slowly.
- Follow the active work area.
- Illuminate a selected object.
- Shift attention between user and object.
- Hold the last valid target briefly during occlusion.
- Return to a neutral pose when attention is lost.
- Refuse targets outside the safe workspace.

## ELEGNT application

Attention should be readable.

For example:

```text
User points to object
    → Orion looks at user’s hand
        → shifts gaze to the object
            → pauses briefly
                → points the light at the object
```

This communicates:

- Orion saw the gesture.
- Orion identified the target.
- Orion is about to act.

## Deliverables

```text
attention controller
target tracker
adaptive-lighting behaviour
joint-attention sequences
lost-target behaviour
workspace and occlusion policy
```

## Exit criteria

- Orion can keep a chosen workspace illuminated.
- Attention transitions are understandable.
- Tracking is smooth rather than twitchy.
- Loss of perception causes predictable behaviour.
- Expressive anticipation does not significantly reduce pointing accuracy.

---

# 20. Milestone 11 — Behaviour Orchestration

## Objective

Coordinate Orion’s capabilities into complete interactions.

## Required behaviour states

```text
booting
idle
resting
attentive
listening
tracking
task_lighting
acknowledging
speaking
notifying
error
sleeping
emergency_stopped
```

## Work required

Implement:

- State transitions.
- Behaviour priorities.
- Capability arbitration.
- Motion interruption.
- User interruption.
- Reactive behaviours.
- Carefully limited proactive behaviours.
- Recovery from perception failure.
- Recovery from audio failure.
- Recovery from interrupted motion.
- Lifecycle management.

A state machine or behaviour tree may be used, but the architecture should be selected based on actual behaviour requirements rather than trend.

## Important rule

Only one subsystem should own the authoritative decision about Orion’s active behaviour.

The motion, lighting, audio, and perception nodes should not independently decide what Orion is “doing.”

## Deliverables

```text
orion_behavior package
behaviour-state model
transition rules
priority and interruption policy
scenario tests
failure-recovery behaviours
```

## Exit criteria

- Orion can execute complete multi-step scenarios.
- Competing requests resolve predictably.
- Emergency stop overrides every behaviour.
- A failed subsystem produces a safe degraded mode.
- Orion’s current state can be inspected externally.

---

# 21. Milestone 12 — Voice and Agent Integration

## Objective

Allow natural interaction without putting language models in the control loop.

## Audio pipeline

```text
microphone
    → wake-word or activation
        → speech-to-text
            → intent interpretation
                → validated Orion capability
                    → behaviour execution
                        → text-to-speech response
```

## Initial voice commands

Start with deterministic commands:

- “Look at me.”
- “Point the light here.”
- “Go to sleep.”
- “Wake up.”
- “Make the light brighter.”
- “Stop.”
- “Return home.”

## Agent layer

A later LLM or agent may:

- Interpret flexible language.
- Choose from approved capabilities.
- Manage conversation.
- Use external tools.
- Select a suitable expression profile.
- Ask for clarification.
- Explain failures.

It must not:

- Publish unrestricted joint trajectories.
- Disable safety checks.
- Override emergency stops.
- Invent physical capabilities.
- Continue moving after losing control authority.

## Multimodal coordination

Voice, light, and movement must be synchronised.

Examples:

- Orient before speaking.
- Pause briefly before answering.
- Nod while acknowledging.
- Lower expressive movement during precise task lighting.
- Stop listening gestures when speech recognition ends.
- Avoid moving continuously during long responses.

ELEGNT’s findings emphasise that movement should align with voice and light rather than operating as an unrelated animation layer.

## Deliverables

```text
orion_audio package
wake-word pipeline
speech-to-text
deterministic intent router
text-to-speech
agent capability interface
voice-motion timing rules
```

## Exit criteria

- Core commands work without an LLM.
- AI failure does not disable normal lamp operation.
- The agent can only invoke approved capabilities.
- “Stop” remains local, immediate, and deterministic.
- Movement and speech timing feel coherent.

---

# 22. Milestone 13 — Context-Aware ELEGNT Expression Engine

## Objective

Move from individually authored expressive animations to a reusable system that can adapt functional movement to context.

## Inputs

The expression engine may receive:

```yaml
functional_goal:
  type: illuminate_target
  target: workpiece

interaction_context:
  mode: task_focused
  user_attention: workpiece
  robot_role: reactive

expression:
  intention: about_to_help
  attention: workpiece
  attitude: confident
  displayed_emotion: calm
  gain: 0.20
```

## Expression dimensions

### Intention

What Orion is about to do.

Possible primitives:

- Anticipatory glance.
- Preparatory lean.
- Movement toward a target before full execution.
- Looking back for confirmation.

### Attention

What Orion is focused on.

Possible primitives:

- User-directed gaze.
- Object-directed gaze.
- Joint attention.
- Alternating gaze between user and target.

### Attitude

Orion’s stance toward the task.

Possible primitives:

- Confident direct motion.
- Hesitant pause.
- Agreement through nodding.
- Disagreement through head shaking.
- Curiosity through head tilt and leaning.

### Apparent emotion

The affective style of the movement.

Possible primitives:

- Calm, slow, smooth movement.
- Excited, broad, quick movement.
- Sad, lowered, compressed posture.
- Surprised recoil.
- Relaxed settling.

## Context-sensitive expression gain

Expression should be reduced during:

- Precision task lighting.
- Fast user corrections.
- Safety recovery.
- High-confidence tracking.
- Emergency conditions.

Expression may be increased during:

- Greeting.
- Acknowledgement.
- Conversation.
- Music.
- Notifications.
- Playful interactions.
- Failure communication.

The paper’s findings suggest social tasks benefit more strongly from expression, while excessive movement during function-focused tasks may become distracting or inefficient.

## Deliverables

```text
expression profiles
spatial motion primitives
temporal motion primitives
expression composer
context-dependent expression gain
functional/expressive A/B framework
user preference settings
```

## Exit criteria

- The same functional goal can be executed with different expression profiles.
- Expression remains subordinate to task and safety constraints.
- Users can identify intended attention and attitude more reliably than with the functional baseline.
- Expressive motion does not materially degrade task accuracy.
- Orion can reduce or disable expression for users who prefer minimal movement.

---

# 23. Milestone 14 — Custom Orion Mechanical and Electrical Design

## Objective

Use evidence from the reference platform to design original Orion hardware.

Do not redesign the mechanism merely because the project has reached a particular date.

Redesign when the current platform has produced evidence about:

- Insufficient joint range.
- Inadequate torque.
- Excessive backlash.
- Excessive noise.
- Poor base stability.
- Cable-routing limitations.
- Camera occlusion.
- Poor light quality.
- Motor heating.
- Difficult maintenance.
- Inability to perform important expressive poses.

## Mechanical work

Design:

- Kinematic layout.
- Number and placement of degrees of freedom.
- Joint ranges.
- Arm lengths.
- Motor mounts.
- Bearing support.
- Counterbalance springs.
- Mechanical stops.
- Base weight and footprint.
- Cable channels.
- Service access.
- Pinch protection.
- Head enclosure.
- Diffuser.
- Camera placement.
- Microphone and speaker placement.

## Electrical work

Design:

- Power architecture.
- Motor rail.
- Compute rail.
- Lighting rail.
- Fuse and protection.
- Emergency-stop circuit.
- Power switch.
- Current sensing.
- Thermal sensing.
- Motor communication bus.
- LED control.
- Camera and audio connections.
- Optional custom PCB.

## Simulation work

Update:

- Fusion assembly.
- Mass properties.
- URDF/Xacro.
- Collision geometry.
- Inertial values.
- MuJoCo model.
- Servo or actuator model.
- Stability tests.

## Deliverables

```text
custom Orion CAD
engineering drawings
torque calculations
mass and centre-of-gravity analysis
updated digital twin
electrical schematic
bill of materials
prototype parts
assembly guide
```

## Exit criteria

- Every major design change is tied to evidence.
- The custom model works in simulation.
- Torque and stability margins are documented.
- Camera, light, and perceived gaze align.
- The design can be assembled and serviced.
- Physical safety is considered before appearance is finalised.

---

# 24. Milestone 15 — Product Hardening, Evaluation, and Release

## Objective

Move from an impressive prototype to a dependable robotic appliance.

## Reliability

Validate:

- Long-duration idle operation.
- Repeated motion cycles.
- Servo temperature.
- Power stability.
- Restart behaviour.
- Controller recovery.
- Sensor disconnection.
- Network loss.
- Audio failure.
- Camera failure.
- Storage corruption.
- Emergency stopping.

## Physical safety

Evaluate:

- Pinch points.
- Tip-over risk.
- Sharp edges.
- Cable fatigue.
- Overcurrent protection.
- Thermal surfaces.
- Mechanical stops.
- Human contact.
- Child and pet interaction assumptions.
- Safe torque disable.

## Privacy and security

Define:

- Whether camera processing is local.
- Whether audio leaves the device.
- Recording indicators.
- Data-retention rules.
- Network authentication.
- Update mechanisms.
- Permissions for agent tools.
- Safe default behaviour when cloud services are unavailable.

## Human evaluation

For each key interaction, compare:

- Functional version.
- Expressive version.

Measure engineering outcomes:

- Pointing error.
- Completion time.
- Maximum velocity.
- Maximum acceleration.
- Maximum jerk.
- Actuator effort.
- Base movement.
- Collision clearance.
- Tracking stability.

Measure user perception:

- What was Orion paying attention to?
- What was Orion about to do?
- Did Orion appear confident or hesitant?
- Was its movement pleasant or distracting?
- Was the behaviour understandable?
- Did the movement match the voice and light?
- Would the user prefer more or less expression?

## Release work

Prepare:

```text
complete documentation
build guide
bill of materials
calibration guide
simulation setup
hardware setup
safety guide
example behaviours
testing instructions
licensing and attribution
known limitations
```

## Exit criteria

- Orion can operate for extended periods without supervision.
- Safety failures lead to deterministic stopped states.
- Functional lamp operation works without cloud services.
- Core interactions have been tested with real users.
- Documentation allows another person to reproduce the system.
- Licensing and source provenance are clear.

---

# 25. Milestone 16 — Optional Advanced Capabilities

These features should only be considered after the core lamp is safe, useful, and reliable.

## Possible extensions

- 3D object scanning.
- Workspace mapping.
- Projection onto the desk.
- Smart-home control.
- Computer and build-status notifications.
- Music-reactive movement.
- Gesture-command vocabulary.
- Multi-user recognition.
- User-specific expression preferences.
- Long-term contextual memory.
- MCP-connected agent tools.
- Learned movement generation.
- Imitation learning.
- Reinforcement learning.
- Additional degrees of freedom.
- Mobile base.
- Touch interaction.

These are not required to prove Orion’s core idea.

A robotic lamp that safely directs light, communicates attention, and moves expressively is already a substantial robotics system.

---

# 26. ELEGNT Behaviour-Design Template

Every important Orion behaviour should be documented using the following structure.

## Scenario

What is happening?

## Functional objective

What physical result must Orion achieve?

## Attention target

What person, object, or area should Orion appear to focus on?

## Intended user inference

What should the user believe Orion has noticed, understood, or intends to do?

## Expression category

Choose one or more:

```text
intention
attention
attitude
apparent emotion
```

## Spatial primitives

Examples:

- Head tilt.
- Lean.
- Approach.
- Recoil.
- Lowered posture.
- Expanded posture.
- Compressed posture.
- User-directed gaze.
- Target-directed gaze.

## Temporal primitives

Examples:

- Anticipation.
- Pause.
- Slow start.
- Decisive acceleration.
- Smooth settling.
- Overshoot.
- Repeated bounce.
- Hesitation.
- Sudden stop.

## Multimodal elements

- Task light.
- RGB light.
- Sound.
- Voice.
- Projection.
- Screen or external UI.

## Hard constraints

- Joint limits.
- Stability.
- Collision.
- Maximum speed.
- Human proximity.
- Target accuracy.
- Time limit.

## Functional baseline

Describe the minimum movement required to complete the task.

## Expressive variation

Describe what is added or changed to communicate state.

## Evaluation

How will functional and expressive performance be compared?

---

# 27. Suggested Repository Evolution

Do not create every package immediately. Add packages when a real architectural boundary appears.

A possible long-term structure is:

```text
orion/
├── runtime/
├── motion/
├── description/
├── simulation/
│   └── mujoco/
│
├── orion_studio/
│
├── hardware/
│   ├── cad/
│   ├── electronics/
│   ├── bom/
│   └── assembly/
│
├── tests/
│
└── docs/
    ├── learning_notes/
    ├── architecture/
    ├── milestones/
    ├── scenarios/
    ├── product/
    └── orion_guidebook.md
```

The immediate project does not need all of these packages.

At the current stage, the important packages are:

```text
orion_description
orion_motion
```

Everything else should be introduced deliberately.

---

# 28. Suggested Orion Versions

## Orion v0.1 — Digital body

**Status: complete**

- URDF.
- MuJoCo.
- Native Rust runtime.
- Semantic joints.
- Valid trajectory command.

## Orion v0.2 — Expressive simulation

- Named poses.
- Keyframe animations.
- Functional/expressive pairs.
- Cross-simulator motion format.
- Safety validation.

## Orion v0.3 — Task-space lamp

- Forward kinematics.
- Inverse kinematics.
- Light-axis targeting.
- Workspace limits.
- Unreachable-target handling.

## Orion v0.4 — Physical reference platform

- LeLamp-compatible hardware.
- Native STS3215 interface.
- Calibration.
- Emergency stop.
- Sim-to-real motion.

## Orion v0.5 — Perceiving lamp

- Camera.
- Depth.
- Hand and user tracking.
- World model.
- Adaptive work-area illumination.

## Orion v0.6 — Multimodal expressive assistant

- Lighting scenes.
- Audio.
- Behaviour orchestration.
- Voice.
- Context-sensitive expression.
- Orion Studio.

## Orion v1.0 — Custom Orion

- Original mechanical design.
- Original electrical architecture.
- Product-quality lighting.
- Quiet and stable motion.
- Integrated sensors.
- Safety hardening.
- User-tested expressive behaviours.
- Reproducible documentation.

---

# 29. Current Orion Position

Orion has completed the motion-foundation phase.

The immediate next milestone is:

> Turn basic keyframe playback into a dependable motion system with smooth,
> safe, interruptible, and simulator-portable trajectory execution.

The required deliverables are:

```text
interpolation library
motion preemption
motion cancellation
trajectory feedback
stability checks
cross-simulator playback tests
motion validation report
```

The motion pipeline must distinguish:

```text
requested keyframes
generated trajectory
validated trajectory
executed trajectory
measured trajectory
```

Do not move to task-space target pointing, voice, cameras, an LLM, or a full
expression optimiser until motion quality, safety, cancellation, and simulator
parity work reliably.

The central mental model for this stage is:

> **A requested motion is not executable merely because its keyframes are
> valid. Orion must generate a continuous trajectory from measured state,
> validate its dynamics and stability, execute it through a controlled
> lifecycle, and compare the measured result with what was requested.**
