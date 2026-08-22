# Orion Product Charter

## Document status

- **Status:** In progress
- **Last updated:** 2026-08-22
- **Milestone:** 0 — Product and Interaction Charter
- **Source:** Confirmed project-owner interview decisions

This charter records the product decisions that have been confirmed so far. It
does not mark Milestone 0 complete. Open decisions and unfinished scenario work
are listed below rather than being filled with assumptions.

## Product statement

Orion is an expressive companion robot that lets people interact with embodied
AI without relying on screens. Its movement, lighting, voice, and attention
should make the AI feel physically present rather than contained in an app.

## Primary audience

Orion is intended for a broad household audience rather than one specialist
group. Its behaviour should be understandable and approachable for different
people who share a home.

## Operating environment

Orion is a stationary tabletop robot. It normally operates from a desk, table,
or similar stable surface. A person may carry it to another suitable surface,
but Orion does not move itself around the home.

Orion has the compact base footprint of a typical desk lamp. Its articulated
arm has a larger working area and may extend across part of a desk for useful
lighting and expressive movement.

The exact reach boundary and rules for moving near people, drinks, fragile
objects, and the edges of surfaces still need to be defined.

## Core experience

Orion's main purpose is companionship through screen-free embodied interaction.
Movement and lighting are parts of its communication, not only mechanical and
decorative features.

Orion combines three lighting roles:

1. Ambient room lighting, which is the priority.
2. Directed task lighting for specific activities.
3. Expressive lighting that supports communication and character.

## Character

Orion should feel:

- Curious.
- Calm.
- Playful.

Its expression should feel mature and controlled. Orion should not feel
intrusive or childish.

The detailed movement, lighting, and voice rules needed to preserve this
character will be developed in `docs/personality/orion_character_guide.md`.

## Reactive and proactive behaviour

Normal spoken interaction begins only after a user addresses Orion. Orion
should not start casual spoken conversations by itself.

Orion may proactively:

- Turn toward someone entering the room.
- Offer a small greeting movement.
- Adjust ambient lighting based on time or household activity.
- Give reminders or notifications.
- React to music or household activity.

These actions must remain unintrusive. The exact limits on frequency,
repetition, interruption, quiet hours, and notification style still need to be
defined. Until notification modes are agreed, permission to give a reminder
does not automatically mean permission to begin a spoken conversation.

## Privacy and local operation

All core Orion capabilities must work locally without an internet connection.
This includes:

- Lighting.
- Movement and stopping.
- Reminders and notifications.
- Voice interaction.
- Perception.
- Music and household-activity reactions.
- Conversation.
- Approved memory.

Raw camera and microphone data may be processed temporarily on the device but
must then be discarded. Orion may retain only:

- Selected household preferences.
- Reminders.
- Memories explicitly approved by a user.

Any retained information must remain on the device. No personal sensor,
conversation, preference, reminder, or memory data should leave Orion.

The controls for reviewing, deleting, and expiring retained information still
need to be designed.

## Confirmed first-version boundaries

The following boundaries are already confirmed:

- Orion is not a mobile robot and does not move itself between surfaces.
- Orion does not depend on cloud services for its core capabilities.
- Orion does not send household data away from the device.
- Orion does not begin casual spoken conversations without being addressed.
- Orion should not use intrusive or childish behaviour to demand attention.

Other possible exclusions, including object manipulation, surveillance,
medical monitoring, childcare, and unrestricted control of other household
devices, have not yet been decided.

## Required scenario work

Milestone 0 requires six interaction scenarios. The interview established some
direction for them, but none has a complete storyboard yet.

| Scenario | Confirmed direction | Still needed |
|---|---|---|
| Task-light positioning | Task lighting supports the primary ambient-light role. | Target, trigger, movement, lighting response, failure, and all four variants. |
| User acknowledgement | Behaviour should be calm, curious, playful, and mature. | Acknowledgement cues and all four variants. |
| Following a hand or work area | No interaction rules confirmed yet. | Purpose, consent, tracking limits, loss handling, and all four variants. |
| Timer or quiet notification | Orion may proactively give reminders and notifications. | Notification modes, quiet hours, repetition rules, and all four variants. |
| Unreachable-target failure | No user-facing response confirmed yet. | Safe stop, explanation, recovery, and all four variants. |
| Social conversation or music | Speech is user-initiated; Orion may react to music unintrusively. | Voice and music boundaries plus all four variants. |

Each scenario still needs:

1. A purely functional version.
2. An expressive version.
3. A reactive version initiated by the user.
4. A proactive version where proactive behaviour is appropriate.

## Decisions still required

Milestone 0 cannot close until the project defines:

- The full first-version out-of-scope list.
- Safe movement boundaries around people and household objects.
- The six complete interaction storyboards.
- When proactive actions may repeat and how users interrupt or disable them.
- Reminder and notification output during the user-initiated-speech rule.
- The exact sensor set required by the approved scenarios.
- User controls for approved memories and stored preferences.
- The detailed Orion character guide.

## Closeout checklist

- [x] Primary audience defined.
- [x] Operating location defined.
- [x] General physical workspace defined.
- [x] Core lighting priorities defined.
- [x] High-level personality defined.
- [x] Allowed proactive behaviours identified.
- [x] Local processing and data-retention direction defined.
- [x] Offline-operation requirement defined.
- [ ] Full first-version exclusions agreed.
- [ ] Six scenario storyboards completed.
- [ ] Functional, expressive, reactive, and proactive variants completed.
- [ ] Character guide completed.
- [ ] Every proposed sensor and feature linked to an approved scenario.
- [ ] Milestone 0 exit criteria reviewed and accepted.
