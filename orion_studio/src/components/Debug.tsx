import { useEffect, useMemo, useState } from "react";
import { Activity, AudioLines, FileText, Move3d } from "lucide-react";
import type { StudioVoice } from "../hooks/useStudioVoice";
import { getRuntimeLogs, releaseMovement, type GatewayConnection } from "../lib/gateway";
import type { GatewayStatus } from "../types";
import "./Settings.css";
import "./Debug.css";
import { FailureNotice } from "./FailureNotice";
import { debugStateLabel } from "../lib/debugState";
import { CameraCard } from "./CameraCard";
import { CharacterPanel, RecorderPanel } from "./Recorder";
import { SectionNav } from "./Settings";
const stateValue = (value: string | undefined | null) => value ? <><span>{debugStateLabel(value)}</span> <code>{value}</code></> : "Unavailable";
const reading = (value: number | undefined, unit: string) => value == null || !Number.isFinite(value) ? "—" : `${value.toFixed(2)} ${unit}`;
const sections: [string, string][] = [["now", "Orion now"], ["character", "Character"], ["voice", "Voice"], ["movement", "Movement"],
  ["camera", "Camera"], ["recordings", "Recordings"], ["logs", "Logs"]];

function Values({ values }: { values: [string, React.ReactNode][] }) {
  return <dl className="debug-values">{values.map(([key, value]) => <div key={key}><dt>{key}</dt><dd>{value ?? "Not loaded"}</dd></div>)}</dl>;
}

export function Debug({ voice, connection, status, onRefresh, onHistory }: { onRefresh: () => Promise<void>; onHistory: () => void; voice: StudioVoice; connection: GatewayConnection | null; status: GatewayStatus | null }) {
  const [releasing,setReleasing] = useState(false);
  const [cameraOverlay, setCameraOverlay] = useState(false);
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
  const [filter,setFilter] = useState("");
  const [error,setError] = useState("");
  const [loading,setLoading] = useState(false);
  useEffect(() => {
    let active = true; setLogs([]); setError(""); setLoading(!!connection);
    if (connection) void getRuntimeLogs(connection).then(value => { if (active) setLogs(value.lines); }).catch(error => { if (active) setError(`Logs unavailable: ${error instanceof Error ? error.message : String(error)} Update the gateway if it does not support logs.`); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  },[connection,refresh]);
  const shownLogs = useMemo(() => { const query = filter.trim().toLowerCase(); return query ? logs.filter(line => line.toLowerCase().includes(query)) : logs; }, [logs, filter]);
  const { snapshot,playback } = voice;
  const mind = status?.mind;
  const tiles: [string, React.ReactNode][] = [
    ["Mode", stateValue(status?.runtime.mode)],
    ["Character", stateValue(status?.character.state)],
    ["Movement", status?.character.active_clip ? stateValue(status.character.active_clip) : "None"],
    ["Next idle", status?.character.next_idle_category ? stateValue(status.character.next_idle_category) : "Not scheduled"],
    ["Torque", status ? status.runtime.torque_enabled ? "On" : "Off" : "Unavailable"],
    ["Energy", mind ? `${Math.round(mind.energy * 100)}%` : "—"],
    ["Mood", mind ? <>{mind.label} <code>{mind.mood.toFixed(2)}</code></> : "—"],
    ["Attention", mind ? mind.attention.kind === "object" ? mind.attention.class : mind.attention.track_id ? `${mind.attention.kind} ${mind.attention.track_id}` : mind.attention.kind : "—"],
  ];
  return <section className="settings-page settings-owner debug-page"><header className="settings-heading"><p className="eyebrow">DEVELOPER TOOLS</p><h1>Debug</h1><p>{connection ? "Runtime readings and voice activity reported by Orion’s Pi." : "Connect Orion to read runtime diagnostics."}</p></header>
    <div className="settings-layout"><SectionNav sections={sections} prefix="debug-" label="Debug sections" />
    <div className="settings-stack">
      <section className="settings-card" id="debug-now"><header><Activity size={20}/><div><h2>Orion now</h2><p>{status?.runtime.build_revision ?? "Build unavailable"}</p></div></header>
        <dl className="debug-tiles">{tiles.map(([key, value]) => <div key={key}><dt>{key}</dt><dd>{value}</dd></div>)}</dl>
        {!mind && <p className="settings-help">Mind state is unavailable.</p>}
      </section>
      <div id="debug-character"><CharacterPanel initiative={status?.initiative} ignored={status?.remarks?.ignored} quiet={status?.remarks?.quiet_seconds} /></div>
      <section className="settings-card" id="debug-voice"><header><AudioLines size={20}/><div><h2>Voice</h2><p>What the voice service is running now.</p></div><button className="quiet-button" type="button" onClick={onHistory}>Conversation history →</button></header>
        <Values values={[["Runtime", snapshot.runtime], ["Microphone", snapshot.deviceLabel], ["Wake model", snapshot.wakeModel], ["Wake threshold", snapshot.wakeThreshold?.toFixed(3)],
          ["Speech to text", snapshot.asrModel], ["Reply model", snapshot.agentModel], ["Effort", snapshot.agentEffort], ["Text to speech", snapshot.ttsModel], ["Playback", stateValue(playback.state)]]} />
        <details className="debug-more"><summary>Saved voice settings</summary>
          <Values values={[["Reply model", voice.settings?.model], ["Effort", voice.settings?.effort], ["ASR setting", voice.settings?.asrModel], ["ASR files", voice.settings?.asrPath || "Managed on the Pi"],
            ["TTS setting", voice.settings?.ttsModel], ["Voice files", voice.settings?.ttsPath || "Managed on the Pi"]]} />
        </details>
        <FailureNotice value={voice.loadError} /><FailureNotice value={voice.listeningError} /><FailureNotice value={voice.saveError} />{snapshot.error && <details><summary>Voice error details</summary><pre>{snapshot.error}</pre></details>}
      </section>
      <section className="settings-card debug-joints" id="debug-movement"><header><Move3d size={20}/><div><h2>Movement</h2><p>Position, speed, electrical load and temperature of each joint.</p></div></header>
        {status?.runtime.joints?.length ? <div className="debug-table"><table><thead><tr>{["Joint","Position","Speed","Current","Voltage","Temperature","Status"].map(label => <th key={label}>{label}</th>)}</tr></thead><tbody>{status.runtime.joints.map(joint => <tr key={joint.name}><th>{joint.name}</th><td>{reading(joint.position_rad,"rad")}</td><td>{reading(joint.velocity_rad_s,"rad/s")}</td><td>{reading(joint.current_ma,"mA")}</td><td>{reading(joint.voltage_v,"V")}</td><td>{reading(joint.temperature_c,"°C")}</td><td>{joint.status ?? "—"}</td></tr>)}</tbody></table></div> : <p>Joint telemetry is unavailable.</p>}
        <div className="debug-torque"><div><strong>Release the joints</strong><p className="settings-help">Turning torque off releases the joints; Orion will no longer hold its position. Support Orion before releasing.{torqueBusy && " Pause Orion’s mode and stop active movement first."}</p></div>
          <button className="quiet-button" disabled={!connection || !status?.runtime.torque_enabled || torqueBusy || releasing} onClick={() => void release()}>{releasing ? "Releasing…" : status && !status.runtime.torque_enabled ? "Torque is off" : "Turn torque off"}</button></div>
        {torqueError && <p role="alert" className="settings-error">{torqueError}</p>}
      </section>
      {status?.runtime.hardware !== "v1" && <div className="debug-camera" id="debug-camera"><label className="debug-camera-toggle"><input type="checkbox" checked={cameraOverlay} onChange={event => setCameraOverlay(event.target.checked)} />Show face boxes</label>
        <CameraCard connection={connection} overlay={cameraOverlay} /></div>}
      <div id="debug-recordings"><RecorderPanel connection={connection} /></div>
      <section className="settings-card" id="debug-logs"><header><FileText size={20}/><div><h2>Logs</h2><p>Most recent 200 entries from the runtime, gateway, and listener.</p></div><button className="quiet-button" disabled={!connection || loading} onClick={() => setRefresh(value => value+1)}>{loading ? "Loading…" : "Refresh logs"}</button></header>
        <label className="debug-log-filter">Show only lines with<input type="search" value={filter} placeholder="For example: wake, servo, error" onChange={event => setFilter(event.target.value)} /></label>
        {error && <p role="status" className="settings-error">{error}</p>}
        <pre className="debug-logs">{shownLogs.join("\n") || (loading ? "Reading logs…" : filter ? "No lines match." : "No log entries available.")}</pre></section>
    </div></div>
  </section>;
}
