import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { BehaviourKey, GatewayConnection } from "../lib/gateway";
import { BehaviourPreference, emptyBehaviours } from "../lib/behaviourPreference";
import { FailureNotice } from "./FailureNotice";
import { SettingToggle } from "./Settings";

const controls: Array<[BehaviourKey, string, string]> = [
  ["object_glances", "Object glances", "Occasionally direct an idle glance toward an object on the desk."],
  ["mutual_gaze", "Mutual gaze", "Acknowledge a sustained look with a small, silent response."],
  ["arrival_notice", "Arrival notice", "Give a silent welcome after the room has been empty for ten minutes."],
];
const noSubscription = () => () => {};
const emptySnapshot = () => emptyBehaviours;
export function BehaviourSettings({ connection, supported }: { connection: GatewayConnection | null; supported: boolean }) {
  const preference = useMemo(() => connection && supported ? new BehaviourPreference(connection) : null, [connection, supported]);
  const state = useSyncExternalStore(preference?.subscribe ?? noSubscription, preference?.snapshot ?? emptySnapshot, preference?.snapshot ?? emptySnapshot);
  useEffect(() => { preference?.start(); return () => preference?.stop(); }, [preference]);
  return <section className="settings-card"><header><div><h2>Character behaviours</h2><p>Optional reactions to Orion’s surroundings. Camera and tracking must be on.</p></div></header>
    {!connection ? <p className="settings-help">Connect Orion to change its behaviours.</p> : !supported ? <p className="settings-help">These behaviours need Orion’s V2 camera.</p> : <>
      {controls.map(([key, label, help]) => <SettingToggle key={key} label={label} help={help} checked={state.values?.[key] ?? false}
        disabled={!state.values || state.pending} onChange={value => void preference?.choose(key, value)} />)}
      {!state.values && !state.error && <p role="status">Loading character behaviours…</p>}
      {state.notice && <p role="status">{state.notice}</p>}<FailureNotice value={state.error} />
    </>}
  </section>;
}
