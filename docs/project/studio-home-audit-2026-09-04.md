# Studio home interface audit — 4 September 2026

## Anti-patterns verdict

The overall composition passes: the robot canvas, asset library and parallel
tracks form a recognisable creative tool, rather than a generic dashboard.
Keep the navy/blue/cyan/violet direction specified in `.impeccable.md`; its
presence alone is not a defect. The weak points are hierarchy, state clarity
and the distance between everyday ownership and expert authoring. Repeated
technical labels and uniformly weighted tool buttons make the screen feel
more like a development surface than an approachable home.

## Executive summary

Nine remaining issues: **five high, four medium, none critical or low**. Three
related problems were addressed while implementing pairing: repeated token
entry, no reconnect after network loss, and collapsed header icons.

The priorities are protecting drafts, making preview/hardware state truthful,
and providing clear execution feedback. Then introduce a calm owner home with
an explicit entrance to the existing authoring workspace.

Evidence: live local browser inspection at 1440×900 and 390×844, keyboard
inspection, accessibility-tree inspection, source review and calculated CSS
contrast. No physical robot operations were triggered. Connected execution
findings below are source-based; they are not claimed as hardware trials.

## High-severity findings

### 1. Drafts disappear when changing selection

- **Location:** [App.tsx](../../orion_studio/src/App.tsx), `chooseAsset`, brand
  button and editor mode tabs.
- **Category:** UX / reliability.
- **Evidence:** selection clones the catalog asset and resets `assetDirty`.
  A scene start-time edit was lost after switching to another expression and
  returning. There is no draft store or unsaved-change decision. The home-labelled brand
  button also loads the initial scene. Scene editing does not maintain a dirty
  indicator consistently.
- **Impact:** a normal navigation action can silently discard authoring work.
- **Proposed fix:** keep per-asset drafts in a local workspace, show a changed
  indicator, and offer restore/discard. Do not use Publish as the only way to
  preserve work. Guard destructive selection changes until draft storage exists.
- **Acceptance:** edit each asset type, switch tabs/assets, return and recover
  the edits; close/reopen Studio and restore a draft.
- **Suggested command:** `/harden`.

### 2. Preview labels imply validation that has not happened

- **Location:** [App.tsx](../../orion_studio/src/App.tsx), `stage-meta` and
  diagnostics; [RobotViewport.tsx](../../orion_studio/src/components/RobotViewport.tsx),
  viewport badge.
- **Category:** UX / state communication.
- **Evidence:** disconnected mode displays “Calibration safe”, “URDF / LIVE
  PREVIEW” and a diagnostics count described as “live joint ranges”. The image
  is an authored anchor pose, not live measured robot telemetry. The scene
  Preview button is disabled until compilation.
- **Impact:** users cannot reliably distinguish a static model, validated
  trajectory and actual robot state.
- **Proposed fix:** use “Static pose”, “Preview ready” and “Robot telemetry” only
  for their corresponding data sources. Show “Connect to validate movement”
  while disconnected. Put Hz, velocity and calibration details in diagnostics.
- **Acceptance:** disconnected, compiling, failed and validated states each
  have distinct, accurate labels; none implies physical clearance validation.
- **Suggested command:** `/clarify`.

### 3. Physical execution has no dedicated visible lifecycle

- **Location:** [App.tsx](../../orion_studio/src/App.tsx), `runOnOrion`, transport
  and character strip.
- **Category:** UX / control feedback.
- **Evidence:** clicking Run reports acceptance in the footer. The transport
  provides no persistent foreground run ID/status/cancel action; the visible
  Stop belongs to Character. Offline Run and Start Character look actionable
  but open connection settings. Edited pose/motion runs are correctly rejected
  until published, although the button does not explain this in advance.
- **Impact:** acceptance can be mistaken for completion, and the appropriate
  stopping control is unclear during a foreground run.
- **Proposed fix:** show pending/running/completed/failed state beside Run and
  a run-specific cancel action. Label Character Stop explicitly. Reflect
  disconnected, unpublished and busy prerequisites before the click.
- **Acceptance:** simulation tests show terminal success/failure and cancel
  tied to the correct run; repeated clicks cannot queue accidental duplicates.
- **Suggested command:** `/harden`, then `/clarify`.

### 4. Important text fails the specified contrast target

- **Location:** [styles.css](../../orion_studio/src/styles.css), `--faint`,
  `.asset-list small`, timeline labels and `.primary-button`.
- **Category:** Accessibility / theming.
- **Evidence:** `#59677a` on `#101722` is **3.12:1**. Primary button text
  `#f7faff` on `#3978d5` is **4.16:1**. These are ordinary small-text uses,
  below WCAG 2.2 AA's 4.5:1 requirement (1.4.3). Muted body text passes at
  6.31:1; the entire palette does not need replacing.
- **Impact:** provenance, timing and the principal action become difficult to
  read, particularly at their current small sizes.
- **Proposed fix:** introduce separate passing text tokens for metadata and
  primary buttons; retain faint colours for nonessential decorative lines.
  Increase the smallest timeline/metadata type before adjusting layout.
- **Acceptance:** measured normal text contrast ≥4.5:1 in default, hover and
  selected states; disabled controls assessed separately.
- **Suggested command:** `/normalize`, `/typeset`.

### 5. Timeline draws unresolved events at misleading positions

- **Location:** [Timeline.tsx](../../orion_studio/src/components/Timeline.tsx),
  `triggerTime(...) ?? 0`, motion width fallback and absolute clip layout.
- **Category:** UX / visual accuracy.
- **Evidence:** the disconnected initial scene places marker-triggered light
  events at zero; the live desktop screenshot shows overlapping light labels.
  An uncompiled motion has zero calculated width and CSS supplies an arbitrary
  minimum. These are missing timings presented as definite positions.
- **Impact:** users see an inaccurate schedule and cannot select/read every
  event easily.
- **Proposed fix:** show unresolved marker events in a labelled pending lane
  until compilation, then stack overlapping events into separate lanes.
  Provide timeline zoom and a selected-event label independent of clip width.
- **Acceptance:** no unknown trigger silently becomes zero; every overlapping
  event remains keyboard-selectable and readable at the minimum zoom.
- **Suggested command:** `/arrange`, `/harden`.

## Medium-severity findings

### 6. The home screen is the complete editor

- **Location:** [App.tsx](../../orion_studio/src/App.tsx), initial scene,
  character strip, inspector and authoring dock.
- **Category:** UX / information hierarchy.
- **Evidence:** the initial page exposes event deletion, deployment smoke,
  idle seed, preview anchor and “Publish v2 asset” alongside everyday voice
  and character controls. The brand's Home action simply selects a scene.
- **Impact:** an owner looking to talk to Orion must interpret an animation
  workstation; authoring vocabulary competes with routine controls.
- **Proposed fix:** introduce Home and Create destinations. Home contains
  connection/character/microphone state, a small expressive robot view and
  curated expressions. Create retains the existing canvas/library/timeline.
  Move seeds, deployment assets and simulated Listen/Think controls to developer
  tools. Keep one shared connection across both destinations.
- **Acceptance:** an owner can connect, enable voice, choose an expression and
  stop the relevant activity without encountering schema or spline terminology.
- **Suggested command:** `/distill`, `/onboard`.

### 7. Compact layouts bury the active task

- **Location:** [styles.css](../../orion_studio/src/styles.css), responsive
  workspace, library, stage and dock rules.
- **Category:** Responsive / accessibility.
- **Evidence:** at 390×844 the document is 1779 px tall; the library and character
  controls occupy much of the first viewport and editing/publishing are below
  the fold. Header labels become icon-only. At 1440×900, document height is
  1003 px, placing footer feedback below the initial viewport. There was no
  document-wide horizontal overflow at the tested widths.
- **Impact:** users repeatedly scroll between editing, playback and feedback.
- **Proposed fix:** use a compact library chooser, collapsible inspector and
  a persistent task/action area. Preserve readable connection status, rather
  than treating a connection icon as sufficient on small screens.
- **Acceptance:** at compact width and 200% text zoom, the current asset,
  connection state and relevant action remain discoverable without overlapping.
  Text zoom remains to be tested; no zoom failure is claimed here.
- **Suggested command:** `/adapt`.

### 8. Navigation semantics exceed keyboard implementation

- **Location:** [App.tsx](../../orion_studio/src/App.tsx), `role="tablist"` and
  `role="tab"`; [VoicePanel.tsx](../../orion_studio/src/components/VoicePanel.tsx).
- **Category:** Accessibility.
- **Evidence:** editor tabs have no roving focus, arrow-key handling or linked
  tab panels. Voice opens a labelled dialog but has no focus transfer/return or
  Escape handler. The pairing panel added in this change does support initial
  focus, Escape and focus return; copy that behaviour consistently.
- **Impact:** keyboard and screen-reader users encounter inconsistent controls.
- **Proposed fix:** implement the complete tabs pattern or use ordinary
  navigation buttons. Standardise nonmodal panels with explicit close, focus
  management and Escape, without trapping focus in a nonmodal surface.
- **Acceptance:** full keyboard navigation and screen-reader review; selected
  asset buttons also communicate selection programmatically.
- **Suggested command:** `/harden`.

### 9. The viewport consumes resources while idle

- **Location:** [RobotViewport.tsx](../../orion_studio/src/components/RobotViewport.tsx),
  render loop and cleanup.
- **Category:** Performance.
- **Evidence:** WebGL renders continuously with shadows even while static;
  cleanup disposes the renderer/controls but does not traverse and dispose
  mesh geometry/materials. The production JS bundle is approximately 1 MB
  before compression, with Vite's large-chunk advisory. CPU/GPU impact was
  not profiled, so no numerical performance cost is claimed.
- **Impact:** likely avoidable GPU work and retained resources on repeated
  viewport construction; unnecessary cost for a simple owner home.
- **Proposed fix:** render on invalidation while static, continue frames only
  during orbit damping/playback, pause hidden views, dispose owned resources,
  and lazy-load the authoring/3D bundle where it improves the Home path.
- **Acceptance:** profiler comparison of idle versus playback; repeated mount/
  unmount shows stable resource usage and preserved orbit position.
- **Suggested command:** `/optimize`.

## Patterns and positive findings

The recurring issue is insufficient separation between authored data, robot
state and developer diagnostics. This affects copy, navigation and timelines.
The visual system itself has a coherent foundation: a dominant robot canvas,
restrained accents, useful tracks, native labelled inputs, visible focus rings,
a skip link and a live-region footer. CSS includes reduced-motion handling.
Edited motion/pose execution already has a publish guard; preserve it.

The pairing implementation also stops rebuilding capabilities on every
heartbeat. The old changing capabilities object recreated the catalog and
viewport repeatedly; the reconnect controller preserves its identity during
ordinary status polling.

## Recommendations by priority

1. **Immediate:** protect drafts and correct preview/execution claims (1–3).
2. **Short term:** fix contrast and unresolved timeline placement (4–5), then
   complete keyboard navigation (8).
3. **Medium term:** introduce Home/Create and adapt the compact workspace
   around task continuity (6–7).
4. **Long term:** profile viewport resource use and split costly views (9).

Start with `/harden` and `/clarify`, use `/normalize` for contrast, then
`/arrange` and `/adapt` for the timeline and compact workspace. This report
proposes the broader interface changes; they have not been implemented.
