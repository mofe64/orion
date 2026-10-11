import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { ArrowUpRight, Info, Mic, Moon, Play, Sparkles, Sun, SunDim } from "lucide-react";
import { restOrion, setCharacterMode, setUserMode, updateRoutines, setLamp, runScene, type GatewayConnection } from "../lib/gateway";
import { createLampCommitter, hueName, lampChannels, lampPreview, type LampMood, type ManualLampSetting } from "../lib/homeLamp";
import { acceptedRun, type TrackedRun } from "./RunFeedback";
import type { GatewayStatus, ProjectCatalog } from "../types";
import "./Home.css";
import { failure, forOwner, OwnerRequestError, type Failure } from "../lib/feedback";
import { FailureNotice } from "./FailureNotice";
import { useScopedFeedback } from "../hooks/useScopedNotice";
import { useTrackingMode } from "../hooks/useTrackingMode";
import { TrackingControl } from "./TrackingControl";

import { assetDisplayName } from "../lib/displayName";
import { orionStateLabel } from "../lib/orionMode";
import { quickExpressions } from "../lib/quickExpressions";
import { ListsCard } from "./ListsCard";
import { CameraCard } from "./CameraCard";
import { AlarmForm } from "./AlarmForm";
import { ReminderForm } from "./ReminderForm";
import { alertTime } from "../lib/alertTime";

const RobotViewport = lazy(() => import("./RobotViewport").then(module => ({ default: module.RobotViewport })));
export const CHARACTER_MODE_FAILURE = "Studio couldn't change Orion's mode. Please try again.";
export const changeCharacterMode = (change: (enabled: boolean) => Promise<void>, enabled: boolean) => forOwner(() => change(enabled), CHARACTER_MODE_FAILURE);

interface Props {
  catalog: ProjectCatalog;
  theme: "dark" | "light";
  voiceLabel: string;
  listening?: boolean;
  voiceAvailable?: boolean;
  connection: GatewayConnection | null;
  status: GatewayStatus | null;
  onConnect: () => void;
  onVoice: () => void;
  onCreate: () => void;
  onDiagnostics?: () => void;
  onRefresh: () => Promise<void>;
  voiceNotice?: string;
  voiceError?: Failure | null;
  onRun: (run: TrackedRun | null) => void;
}

type FeedbackArea = "mode" | "lamp" | "expression" | "alert";
export function homeFeedbackArea(label: string): FeedbackArea {
  if (["Idle mode", "Lamp mode", "Pause mode", "Resume mode", "Go to rest"].includes(label)) return "mode";
  if (["Warm white light", "Custom color", "Light off"].includes(label)) return "lamp";
  if (["Stop alarm", "Cancel alert", "Cancel alert series", "New reminder"].includes(label)) return "alert";
  return "expression";
}
export function Home({ catalog, theme, voiceLabel, listening = false, voiceAvailable = false, connection, status, onConnect, onVoice, onCreate, onDiagnostics, onRefresh, voiceNotice, voiceError, onRun }: Props) {
  const trackingSupported = status?.runtime.hardware !== "v1";
  const tracking = useTrackingMode(connection, trackingSupported);
  const trackingControl = <TrackingControl {...tracking} connected={!!connection} supported={trackingSupported}
    runtimeFailure={status?.character.hands_failure?.message}
    onChange={mode => void tracking.choose?.(mode)} />;
  const [pending, setPending] = useState<string | null>(null);
  const [mood, setMood] = useState<LampMood>("Warm white");
  const [hue, setHue] = useState(210);
  const [brightness, setBrightness] = useState(40);
  const [brightnessChosen, setBrightnessChosen] = useState(false);
  const lampDirty = useRef(false);
  const [appliedLamp, setAppliedLamp] = useState<ManualLampSetting | null>(null);
  const lightOn = status?.rest?.light_on ?? appliedLamp?.enabled ?? false;
  const resting = status?.rest?.state === "resting" || status?.rest?.state === "going_to_rest";
  const previewLight = useMemo(() => appliedLamp ? lampPreview({ ...appliedLamp,
    enabled: status?.rest ? lightOn : appliedLamp.enabled }) : { red: 0, green: 0, blue: 0, white: 0 }, [appliedLamp, lightOn, status?.rest]);
  const [result, setResult] = useScopedFeedback<{ text: string; area: FeedbackArea; failure?: Failure } | null>("home", connection);
  const feedback = (area: FeedbackArea) => {
    if (!pending && result?.area === area && result.failure) return <FailureNotice value={result.failure} />;
    const text = pending && homeFeedbackArea(pending) === area ? `${pending}…` : result?.area === area ? result.text : "";
    return text ? <p className="oh-feedback" role="status"><Info size={14} /><span>{text}</span></p> : null;
  };
  const request = useRef<symbol | null>(null);

  useEffect(() => {
    // Legacy gateways only report the last requested lamp setting:
    // The gateway has no lamp telemetry. Never carry a previous session's command into a new connection.
    // Rest-aware gateways add effective power, but still do not report saved color preferences.
    request.current = null;
    setPending(null); setAppliedLamp(null); setResult(null);
    setBrightness(40); setBrightnessChosen(false); setMood("Warm white"); setHue(210); lampDirty.current = false;
    return () => { request.current = null; };
  }, [connection]);

  const act = async (label: string, work: (value: GatewayConnection) => Promise<unknown>, onAccepted?: () => void) => {
    if (!connection || request.current) return false;
    let succeeded = false;
    const id = Symbol(label);
    request.current = id;
    setPending(label); setResult(null);
    try {
      const response = await work(connection);
      if (request.current !== id) return true;
      succeeded = true;
      onAccepted?.();
      const run = acceptedRun(response, label === "Go to rest" ? "movement" : "scene", label);
      if (run) onRun(run);
      setResult({ text: `${label} accepted.`, area: homeFeedbackArea(label) });
      try { await onRefresh(); }
      catch { if (request.current === id) setResult({ text: `${label} accepted. Status refresh unavailable.`, area: homeFeedbackArea(label) }); }
    } catch (error) {
      if (request.current !== id) return true;
      const message = error instanceof Error ? error.message : String(error);
      setResult({ text: message, area: homeFeedbackArea(label), ...(error instanceof OwnerRequestError ? { failure: failure(message, error) } : {}) });
    } finally {
      if (request.current === id) { request.current = null; setPending(null); void commitLamp.flush(); }
    }
    return succeeded;
  };
  const applyLight = (setting: ManualLampSetting) => {
    return act(setting.enabled ? setting.mood === "Warm white" ? "Warm white light" : "Custom color" : "Light off",
      value => setLamp(value, lampChannels(setting)), () => setAppliedLamp(setting));
  };
  // The queued callback receives a complete setting, so it never reads an old slider value.
  // Recreating/cancelling on connection change prevents a release reaching another Orion.
  const commitLamp = useMemo(() => createLampCommitter(applyLight), [connection]);
  useEffect(() => () => commitLamp.cancel(), [commitLamp]);
  const releaseLight = () => {
    if (!lampDirty.current) return;
    lampDirty.current = false;
    commitLamp.queue({ enabled: true, mood, brightness, hue });
  };
  const disabled = !connection || pending !== null;
  const userMode = status?.routines?.mode ?? status?.rest?.mode ?? "idle";
  const ringing = status?.routines?.ringing ?? false;
  const activeAlerts = status?.routines?.alerts.filter(alert => ["pending", "ringing", "announcing"].includes(alert.state) || alert.kind === "reminder" && alert.state === "missed" && Date.now()/1000 - alert.due_unix <= 86400) ?? [];
  const foregroundBusy = ringing || !!(status?.scene.active || status?.speech.active || (status?.runtime.motion && !status.runtime.motion.name?.startsWith("idle_")));
  const characterLabel = orionStateLabel(status);
  if (!connection) return <section className="home-dashboard" id="workspace" aria-label="Orion home">
    <header className="oh-heading"><div><h1>Your Orion awaits.</h1><p>Connect Orion to talk, choose scenes, and control its light.</p></div></header>
    <div className="oh-disconnected"><p>Your scenes and previews are available in Animation.</p><button className="oh-connect" onClick={onConnect}>Connect Orion <ArrowUpRight size={16} /></button></div>
    {trackingControl}
  </section>;

  return <section className="home-dashboard" id="workspace" aria-label="Orion home">
    <header className="oh-heading"><div><h1>Orion is connected.</h1><p>Talk, play a scene, or set the light.</p></div>
    </header>
    <div className="oh-layout">
      <div className="oh-left">
        <section className="oh-robot" aria-label="Orion mode and preview">
          <header className="oh-panel-heading"><h2>Your Orion</h2><div className="oh-panel-tools"><span className="oh-state">{characterLabel}</span><button className="oh-chip-button" disabled={disabled || !status} onClick={() => void act(status?.character.enabled ? "Pause mode" : "Resume mode", value => changeCharacterMode(async enabled => { await setCharacterMode(value, enabled); }, !status?.character.enabled))}>{status?.character.enabled ? "Pause mode" : "Resume mode"}</button></div></header>
          <div className="oh-model"><Suspense fallback={<p className="oh-model-loading" role="status">Loading Orion’s 3D model…</p>}>
            <RobotViewport catalog={catalog} joints={(catalog.poses.home ?? catalog.poses.attentive).positions} light={previewLight} mode="home" theme={theme} />
          </Suspense></div>
          <p className="oh-model-caption">Home pose · Model preview, not live position</p>
          <div className="oh-mode-row"><div className="oh-modes" role="group" aria-label="Orion mode">
            <button className="oh-mode" disabled={disabled || ringing} aria-pressed={!!status?.character.enabled && userMode === "idle"} onClick={() => void act("Idle mode", value => setUserMode(value, "idle"))}><Sparkles size={18} /><span><strong>Idle mode</strong><small>Rest after 30 minutes</small></span></button>
            <button className="oh-mode" disabled={disabled || ringing} aria-pressed={!!status?.character.enabled && userMode === "lamp"} onClick={() => void act("Lamp mode", value => setUserMode(value, "lamp"))}><Sun size={18} /><span><strong>Lamp mode</strong><small>Stay on with animations</small></span></button>
          </div>
          <div className="oh-rest-action">
            <button className="oh-mode" disabled={disabled || resting || status?.runtime.motion?.name === "rest"} onClick={() => void act("Go to rest", restOrion)}><Moon size={18} /><span><strong>Go to rest</strong><small>Gently settle down</small></span></button>
          </div></div>
          {feedback("mode")}
        </section>
        <div className="oh-inputs"><div className="oh-input-control">
          <div className="oh-talk"><span className="oh-mic"><Mic size={19} /></span><span><strong>Listening</strong><small>{voiceLabel}</small></span><button className="studio-switch" role="switch" aria-label="Orion listening" aria-checked={listening} disabled={!voiceAvailable} onClick={onVoice}><span /></button></div>
          {voiceNotice && <p role="status" className="oh-feedback">{voiceNotice}</p>}<FailureNotice value={voiceError ?? null} />
        </div>{trackingControl}</div>
      </div>
      <section className="oh-lamp" aria-label="Lamp controls" aria-busy={pending !== null}>
        <header className="oh-panel-heading"><h2>Lamp</h2><button className="oh-switch" role="switch" aria-label="Lamp power" aria-checked={lightOn} aria-describedby="lamp-command-state lamp-command-help" disabled={disabled || resting} onClick={() => { commitLamp.cancel(); lampDirty.current = false; applyLight({ enabled: !lightOn, mood, brightness, hue }); }}><span /></button></header>
        <p id="lamp-command-state">{!connection ? "Connect to control the light" : resting ? (lightOn ? "Fading off for rest" : "Off while resting · lamp setting saved") : status?.rest ? `Light ${lightOn ? "on" : "off"}` : appliedLamp ? `Last set: ${appliedLamp.enabled ? `${appliedLamp.mood} · On` : "Off"}` : "Not set in this session"}</p>
        <div className="oh-dial">
          <svg viewBox="0 0 216 216" aria-hidden="true"><circle className="oh-dial-track" cx="108" cy="108" r="91" fill="none" strokeWidth="8" strokeLinecap="round" strokeDasharray="429 572" />{(brightnessChosen || appliedLamp) && <circle className="oh-dial-value" cx="108" cy="108" r="91" fill="none" strokeWidth="8" strokeLinecap="round" strokeDasharray={`${429 * brightness / 100} 572`} />}</svg>
          <div><output htmlFor="lamp-brightness">{brightnessChosen || appliedLamp ? <>{brightness}<small>%</small></> : "–"}</output><label htmlFor="lamp-brightness">{brightnessChosen || appliedLamp ? "Brightness" : "Not set in this session"}</label></div>
        </div>
        <div className="oh-range"><SunDim size={18} /><input id="lamp-brightness" type="range" min="1" max="100" value={brightness} disabled={!connection} aria-describedby="lamp-brightness-help" aria-valuetext={brightnessChosen || appliedLamp ? `${brightness}%` : "Choose brightness; Orion does not report its current level"} onChange={event => { setBrightness(Number(event.target.value)); setBrightnessChosen(true); lampDirty.current = true; }} onPointerUp={releaseLight} onKeyUp={releaseLight} onBlur={releaseLight} /><Sun size={18} /></div>
        <fieldset className="oh-light-moods"><legend>Light mood</legend><div>{(["Warm white", "Custom color"] as const).map(value => <button key={value} type="button" aria-pressed={mood === value} disabled={disabled} onClick={() => { setMood(value); commitLamp.queue({ enabled: true, mood: value, brightness, hue }); }}>{value}</button>)}</div></fieldset>
        {mood === "Custom color" && <div className="oh-color"><div><label htmlFor="lamp-color">Choose your color</label><span className="oh-swatch" style={{ background: `hsl(${hue} 75% 65%)` }} aria-hidden="true" /></div><input className="orion-hue-slider" id="lamp-color" type="range" min="0" max="360" value={hue} aria-valuetext={hueName(hue)} disabled={!connection} onChange={event => { setHue(Number(event.target.value)); lampDirty.current = true; }} onPointerUp={releaseLight} onKeyUp={releaseLight} onBlur={releaseLight} /></div>}
        {feedback("lamp")}
        <p className="oh-note" id="lamp-brightness-help">Changes apply when you let go and turn the lamp on. Orion does not report its brightness, so the dial shows what you set here.</p>
        <p className="oh-note" id="lamp-command-help">{status?.rest ? "Rest keeps the light off and saves your setting." : "Idle mode, Lamp mode and speech can briefly take over the light."}</p>
      </section>
    </div>
    <section className="oh-expressions" aria-label="Expressions"><header className="oh-panel-heading"><h2>Expressions</h2><button className="oh-text-button" onClick={onCreate}>See all <ArrowUpRight size={15} /></button></header>
      <div className="oh-expression-list">{quickExpressions(catalog).map(({name}) => <button key={name} disabled={disabled || foregroundBusy} onClick={() => void act(assetDisplayName(name), value => runScene(value, name))}><span className="oh-expression-icon"><Sparkles size={16} /></span>{assetDisplayName(name)}<Play className="oh-arrow" size={14} /></button>)}</div>
      {feedback("expression")}
    </section>
    <div className="oh-secondary">
      <div className="oh-column">
        {status?.routines && <section className="oh-card oh-alerts" aria-label="Timers, alarms and reminders">
          <header className="oh-panel-heading"><h2>Timers, alarms and reminders</h2>{ringing && <button className="quiet-button" disabled={disabled} onClick={() => void act("Stop alarm", value => updateRoutines(value, { action: "stop" }))}>Stop sound</button>}</header>
          {ringing && <p role="alert">Say “Hey Orion” to stop the sound. It stops automatically after five minutes.</p>}
          {activeAlerts.length ? <ul>{activeAlerts.map(alert => <li key={alert.id}><span>{alert.label || (alert.kind === "timer" ? "Timer" : "Alarm")} · {alert.state === "ringing" ? "Ringing" : alert.state === "missed" ? "Missed" : alert.state === "announcing" ? "Announcing" : alertTime(alert.due_unix)}{alert.repeat && ` · ${alert.repeat.days.join(", ")} at ${alert.repeat.time}`}</span>{alert.state !== "missed" && <button className="quiet-button" disabled={disabled} aria-label={`Cancel ${alert.label || `${alert.kind} ${alert.id}`}`} onClick={() => void act("Cancel alert", value => updateRoutines(value, { action: "cancel", id: alert.id }))}>{alert.repeat ? "Cancel occurrence" : "Cancel"}</button>}{alert.repeat && alert.state !== "missed" && <button className="quiet-button" disabled={disabled} aria-label={`Cancel series ${alert.label || alert.id}`} onClick={() => void act("Cancel alert series", value => updateRoutines(value, { action: "cancel", id: alert.id, series: true }))}>Cancel series</button>}</li>)}</ul> : <p>Ask Orion to set a timer or an alarm.</p>}
          <details className="oh-disclosure"><summary>New alarm</summary><AlarmForm disabled={disabled} onSave={request => act("New alarm", value => updateRoutines(value, request))} /></details>
          <details className="oh-disclosure"><summary>New reminder</summary><ReminderForm disabled={disabled} onSave={request => act("New reminder", value => updateRoutines(value, request))} /></details>
          {status.routines.error && <p role="alert">{status.routines.error}</p>}
          {feedback("alert")}
        </section>}
        <ListsCard connection={connection} />
      </div>
      {trackingSupported && <CameraCard connection={connection} overlay={tracking.mode === "faces" || tracking.mode === "hands"} onEnabledChange={tracking.cameraChanged} />}
    </div>
    {onDiagnostics && <div className="oh-quick-actions"><button className="quiet-button" onClick={onDiagnostics}>Diagnostics</button></div>}
    {status?.rest?.error && <p className="oh-feedback" role="alert">{status.rest.error}</p>}
  </section>;
}
