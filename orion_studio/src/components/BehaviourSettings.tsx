import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { BehaviourKey, GatewayConnection } from "../lib/gateway";
import { BehaviourPreference, emptyBehaviours } from "../lib/behaviourPreference";
import { QuirkPreference, emptyQuirks } from "../lib/quirkPreference";
import { FailureNotice } from "./FailureNotice";
import { DanceSetting } from "./DanceSetting";
import { SettingToggle } from "./Settings";
import { CommentSetting } from "./CommentSetting";
import { QuirkSetting } from "./QuirkSetting";

const controls: Array<[Exclude<BehaviourKey,"move_to_music"|"proactive_speech"|"demo_mode">, string, string]> = [
  ["mind_idle_rhythm", "Follow shared energy", "Adjust idle timing, movement and background light with Orion’s energy."],
  ["listen_lean_in", "Listening lean", "Lean gently toward the speaker and release when processing starts."],
  ["thinking_beats", "Thinking beats", "Use occasional small gestures during a longer thought."],
  ["wake_sleep_acts", "Wake and sleep gestures", "Stretch after waking and settle before resting."],
  ["posture_drift", "Posture changes", "Occasionally change the held posture while idle."],
  ["act_sounds", "Rare gesture sounds", "Add quiet wake, sleep and yawn sounds outside quiet hours."],
  ["object_glances", "Object glances", "Occasionally direct an idle glance toward an object on the desk."],
  ["mutual_gaze", "Mutual gaze", "Acknowledge a sustained look with a small response."],
  ["arrival_notice", "Arrival notice", "Give a welcome after the room has been empty for ten minutes, and watch people leave."],
  ["new_object_peek", "Object peeks", "Take one curious look at a newly noticed object or pet."],
  ["presence_defers_rest", "Stay awake with company", "Delay automatic rest while a face or hands remain in view."],
];
const noSubscription = () => () => {};
const emptySnapshot = () => emptyBehaviours;
const emptyQuirkSnapshot = () => emptyQuirks;

/** One reader of Orion's behaviour settings, shared by every card on the page. */
export function useBehaviourPreference(connection: GatewayConnection | null, active: boolean) {
  const preference = useMemo(() => connection ? new BehaviourPreference(connection) : null, [connection]);
  const state = useSyncExternalStore(preference?.subscribe ?? noSubscription, preference?.snapshot ?? emptySnapshot, preference?.snapshot ?? emptySnapshot);
  useEffect(() => { if (active) preference?.start(); return () => preference?.stop(); }, [preference, active]);
  return { preference, state };
}
export type Behaviours = ReturnType<typeof useBehaviourPreference>;

/** How lively Orion is and the quirks that make this lamp different. */
export function CharacterSettings({ connection, supported, active = true, behaviours }: { connection: GatewayConnection | null; supported: boolean; active?: boolean; behaviours: Behaviours }) {
  const quirks = useMemo(() => connection ? new QuirkPreference(connection) : null, [connection]);
  const quirkState = useSyncExternalStore(quirks?.subscribe ?? noSubscription, quirks?.snapshot ?? emptyQuirkSnapshot, quirks?.snapshot ?? emptyQuirkSnapshot);
  useEffect(() => { if (active) void quirks?.start(); return () => quirks?.stop(); }, [quirks, active]);
  const { preference, state } = behaviours;
  if (!connection) return <p className="settings-help">Connect Orion to change its character.</p>;
  return <section className="profile-subsection character-settings" aria-label="Character">
    {supported && state.values && <CommentSetting proactive={state.values.proactive_speech} demo={state.values.demo_mode} disabled={state.pending}
      onChange={patch => void preference?.chooseMany(patch)}/>}
    {!supported && <p className="settings-help">How lively Orion is needs Orion’s V2 camera.</p>}
    {quirkState.values && <>
      <p className="settings-help">Every Orion gets its own quirks when it is first set up. Change them here if you like.</p>
      <QuirkSetting quirk="curiosity" value={quirkState.values.curiosity} disabled={quirkState.pending} onChange={value => void quirks?.choose("curiosity", value)}/>
      <QuirkSetting quirk="chattiness" value={quirkState.values.chattiness} disabled={quirkState.pending} onChange={value => void quirks?.choose("chattiness", value)}/>
    </>}
    {(!state.values || !quirkState.values) && !state.error && !quirkState.error && <p role="status">Loading Orion’s character…</p>}
    {quirkState.notice && <p role="status">{quirkState.notice}</p>}<FailureNotice value={quirkState.error} />
  </section>;
}

export function BehaviourSettings({ connection, supported, active = true, behaviours }: { active?: boolean; connection: GatewayConnection | null; supported: boolean; behaviours?: Behaviours }) {
  const own = useBehaviourPreference(behaviours ? null : connection, active);
  const { preference, state } = behaviours ?? own;
  return <section className="settings-card"><header><div><h2>Character behaviours</h2><p>Choose how Orion moves and responds. Visual behaviours also need the camera and tracking.</p></div></header>
    {!connection ? <p className="settings-help">Connect Orion to change its behaviours.</p> : <>
      <DanceSetting value={state.values?.move_to_music} disabled={state.pending} onChange={mode=>void preference?.choose("move_to_music",mode)}/>
      {!supported && <p className="settings-help">The other behaviours need Orion’s V2 camera.</p>}
      {supported && <details className="behaviour-advanced"><summary>Advanced: single behaviours</summary>
        <p className="settings-help">Lively and Demo turn on what they need. Turn single behaviours on or off here.</p>
        {controls.map(([key, label, help]) => <SettingToggle key={key} label={label} help={help} checked={state.values?.[key] ?? false}
          disabled={!state.values || state.pending} onChange={value => void preference?.choose(key, value)} />)}
      </details>}
      {!state.values && !state.error && <p role="status">Loading character behaviours…</p>}
      {state.notice && <p role="status">{state.notice}</p>}<FailureNotice value={state.error} />
    </>}
  </section>;
}
