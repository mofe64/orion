import { useEffect, useState } from "react";
import { Activity, AudioLines, FileText } from "lucide-react";
import type { StudioVoice } from "../hooks/useStudioVoice";
import { getRuntimeLogs, releaseMovement, type GatewayConnection } from "../lib/gateway";
import type { GatewayStatus } from "../types";
import "./Settings.css";
const reading = (value: number | undefined, unit: string) => value == null || !Number.isFinite(value) ? "—" : `${value.toFixed(2)} ${unit}`;
export function Debug({ voice, connection, status, onRefresh }: { onRefresh: () => Promise<void>; voice: StudioVoice; connection: GatewayConnection | null; status: GatewayStatus | null }) {
  const [releasing,setReleasing] = useState(false);
  const [torqueError,setTorqueError] = useState("");
  const torqueBusy = !!(status?.character.enabled || status?.runtime.motion || status?.scene.active || status?.runtime.mode === "moving");
  const release = async () => {
    if (!connection || !status?.runtime.torque_enabled || torqueBusy || releasing) return;
    setReleasing(true); setTorqueError("");
    try { await releaseMovement(connection); await onRefresh(); }
    catch(error) { setTorqueError(error instanceof Error ? error.message : String(error)); }
    finally { setReleasing(false); }
  };
  const [refresh,setRefresh] = useState(0);
  const [logs,setLogs] = useState<string[]>([]);
  const [error,setError] = useState("");
  const [loading,setLoading] = useState(false);
  useEffect(() => {
    let active = true; setLogs([]); setError(""); setLoading(!!connection);
    if (connection) void getRuntimeLogs(connection).then(value => { if (active) setLogs(value.lines); }).catch(error => { if (active) setError(`Logs unavailable: ${error instanceof Error ? error.message : String(error)} Update the gateway if it does not support logs.`); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  },[connection,refresh]);
  const { snapshot,playback } = voice;
  return <section className="settings-page"><header className="settings-heading"><p className="eyebrow">DEVELOPER TOOLS</p><h1>Debug</h1><p>{connection ? "Live readings from Orion and the local voice worker." : "Connect Orion to read runtime diagnostics."}</p></header>
    <div className="settings-columns"><section className="settings-card"><header><Activity size={20}/><div><h2>Robot runtime</h2><p>{status?.runtime.build_revision ?? "Unavailable"}</p></div></header><dl className="debug-values">{Object.entries({ Mode:status?.runtime.mode, Torque:status ? status.runtime.torque_enabled ? "On" : "Off" : undefined, Character:status?.character.state, "Active clip":status?.character.active_clip ?? "None", "Next idle":status?.character.next_idle_category ?? "Not scheduled" }).map(([key,value]) => <div key={key}><dt>{key}</dt><dd>{value ?? "Unavailable"}</dd></div>)}</dl><div className="debug-torque"><p className="settings-help">Turning torque off releases the joints; Orion will no longer hold its position. Support the lamp before releasing.</p>{torqueBusy && <p className="settings-help">Stop character mode and active movement before releasing torque.</p>}<button className="quiet-button" disabled={!connection || !status?.runtime.torque_enabled || torqueBusy || releasing} onClick={() => void release()}>{releasing ? "Releasing…" : status && !status.runtime.torque_enabled ? "Torque is off" : "Turn torque off"}</button>{torqueError && <p role="alert" className="settings-error">{torqueError}</p>}</div></section>
    <section className="settings-card"><header><AudioLines size={20}/><div><h2>Voice</h2><p>{voice.label}</p></div></header>{snapshot.error && <p role="alert" className="settings-error">{snapshot.error}</p>}<dl className="debug-values">{Object.entries({ Runtime:snapshot.runtime, Microphone:snapshot.deviceLabel, "Wake model":snapshot.wakeModel, "Wake threshold":snapshot.wakeThreshold?.toFixed(3), "Speech to text":snapshot.asrModel, "Reply model":snapshot.agentModel, Effort:snapshot.agentEffort, "Text to speech":snapshot.ttsModel, Playback:playback.state }).map(([key,value]) => <div key={key}><dt>{key}</dt><dd>{value ?? "Not loaded"}</dd></div>)}</dl>
    {snapshot.transcript && <p><strong>Latest command</strong><br/>{snapshot.transcript}</p>}{snapshot.response && <p><strong>Orion’s reply</strong><br/>{snapshot.response}</p>}
    <details><summary>Voice timing</summary><dl className="debug-values">{Object.entries({ ...snapshot.latency, "Chunk upload round trips (sum)":playback.uploadMs, "First chunk accepted → player start":playback.firstPlaybackMs, "Pi run elapsed":playback.elapsedMs }).filter(([,value]) => value != null).map(([key,value]) => <div key={key}><dt>{key}</dt><dd>{Math.round(value!)} ms</dd></div>)}</dl><p className="settings-help">Durations use each process’s monotonic clock. Player start is software timing, not measured speaker output. Stages overlap; these values do not add up to end-to-end latency.</p></details></section></div>
    <section className="settings-card debug-joints"><header><Activity size={20}/><div><h2>Joint readings</h2><p>Position, speed, electrical load, and temperature reported by Orion.</p></div></header>{status?.runtime.joints?.length ? <div className="debug-table"><table><thead><tr>{["Joint","Position","Speed","Current","Voltage","Temperature","Status"].map(label => <th key={label}>{label}</th>)}</tr></thead><tbody>{status.runtime.joints.map(joint => <tr key={joint.name}><th>{joint.name}</th><td>{reading(joint.position_rad,"rad")}</td><td>{reading(joint.velocity_rad_s,"rad/s")}</td><td>{reading(joint.current_ma,"mA")}</td><td>{reading(joint.voltage_v,"V")}</td><td>{reading(joint.temperature_c,"°C")}</td><td>{joint.status ?? "—"}</td></tr>)}</tbody></table></div> : <p>Joint telemetry is unavailable.</p>}</section>
    <section className="settings-card"><header><FileText size={20}/><div><h2>Orion service logs</h2><p>Most recent 200 entries from the runtime, gateway, and listener.</p></div><button className="quiet-button" disabled={!connection || loading} onClick={() => setRefresh(value => value+1)}>{loading ? "Loading…" : "Refresh logs"}</button></header>{error && <p role="status" className="settings-error">{error}</p>}<pre className="debug-logs">{logs.join("\n") || (loading ? "Reading logs…" : "No log entries available.")}</pre></section>
  </section>;
}
