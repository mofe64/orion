import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { ArrowUpRight, Info, Mic, Moon, MoveUpLeft, MoveUpRight, Play, Plus, Sparkles, Sun, SunDim } from "lucide-react";
import { restOrion, setCharacterMode, setLamp, runScene, type GatewayConnection } from "../lib/gateway";
import { hueName, lampChannels, lampPreview, type LampMood, type ManualLampSetting } from "../lib/homeLamp";
import { acceptedRun, type TrackedRun } from "./RunFeedback";
import type { GatewayStatus, ProjectCatalog } from "../types";
import "./Home.css";

const RobotViewport = lazy(() => import("./RobotViewport").then(module => ({ default: module.RobotViewport })));
const INITIAL_LAMP: ManualLampSetting = { enabled: true, mood: "Warm white", brightness: 40, hue: 210 };

interface Props {
  catalog: ProjectCatalog;
  theme: "dark" | "light";
  voiceLabel: string;
  connection: GatewayConnection | null;
  status: GatewayStatus | null;
  onConnect: () => void;
  onVoice: () => void;
  onCreate: () => void;
  onRefresh: () => Promise<void>;
  onNotice: (message: string) => void;
  onRun: (run: TrackedRun | null) => void;
}

export function Home({ catalog, theme, voiceLabel, connection, status, onConnect, onVoice, onCreate, onRefresh, onNotice, onRun }: Props) {
  const [pending, setPending] = useState<string | null>(null);
  const [mood, setMood] = useState<LampMood>("Warm white");
  const [hue, setHue] = useState(210);
  const [brightness, setBrightness] = useState(40);
  const [appliedLamp, setAppliedLamp] = useState<ManualLampSetting | null>(null);
  const previewLight = useMemo(() => lampPreview(appliedLamp ?? INITIAL_LAMP), [appliedLamp]);
  const [result, setResult] = useState("");
  const request = useRef<symbol | null>(null);

  useEffect(() => {
    // The gateway has no lamp telemetry. Never carry a previous session's command into a new connection.
    request.current = null;
    setPending(null); setAppliedLamp(null); setResult("");
    return () => { request.current = null; };
  }, [connection]);

  const act = async (label: string, work: (value: GatewayConnection) => Promise<unknown>, onAccepted?: () => void) => {
    if (!connection || request.current) return;
    const id = Symbol(label);
    request.current = id;
    setPending(label); setResult("");
    try {
      const response = await work(connection);
      if (request.current !== id) return;
      onAccepted?.();
      const run = acceptedRun(response, label === "Go to rest" ? "movement" : "scene", label);
      if (run) onRun(run);
      setResult(`${label} accepted.`); onNotice(`${label} accepted.`);
      try { await onRefresh(); }
      catch { if (request.current === id) { setResult(`${label} accepted. Status refresh unavailable.`); onNotice(`${label} accepted. Status refresh unavailable.`); } }
    } catch (error) {
      if (request.current !== id) return;
      const message = error instanceof Error ? error.message : String(error);
      setResult(message); onNotice(message);
    } finally {
      if (request.current === id) { request.current = null; setPending(null); }
    }
  };
  const applyLight = (enabled: boolean) => {
    const setting = { enabled, mood, brightness, hue };
    void act(enabled ? mood === "Warm white" ? "Warm white light" : "Custom color" : "Light off",
      value => setLamp(value, lampChannels(setting)), () => setAppliedLamp(setting));
  };
  const disabled = !connection || pending !== null;
  const foregroundBusy = !!(status?.scene.active || status?.speech.active || (status?.runtime.motion && !status.runtime.motion.name?.startsWith("idle_")));
  const characterLabel = !connection ? "Disconnected" : !status ? "Awaiting status" : status.character.enabled
    ? `Character · ${status.character.state.replaceAll("_", " ")}` : "Character off";

  return <section className="home-dashboard" id="workspace" aria-label="Orion home">
    <header className="oh-heading"><div><h1>Hello, Orion.</h1><p>Talk, choose an expression, or set the light.</p></div>
      {!connection && <button className="oh-connect" onClick={onConnect}>Connect Orion <ArrowUpRight size={16} /></button>}
    </header>
    <div className="oh-layout">
      <div className="oh-left">
        <section className="oh-robot" aria-label="Orion character">
          <header className="oh-panel-heading"><h2>Your Orion</h2><span className="oh-state">{characterLabel}</span></header>
          <div className="oh-model"><Suspense fallback={<p className="oh-model-loading" role="status">Loading Orion’s 3D model…</p>}>
            <RobotViewport catalog={catalog} joints={catalog.poses.attentive.positions} light={previewLight} mode="home" theme={theme} />
          </Suspense></div>
          <p className="oh-model-caption">Attentive pose · Model preview, not live position</p>
          <div className="oh-modes">
            <button className="oh-mode" disabled={disabled || status?.character.enabled} aria-pressed={!!status?.character.enabled} onClick={() => void act("Character mode", value => setCharacterMode(value, true))}><Sparkles size={18} /><span><strong>Character mode</strong><small>A little personality</small></span></button>
            <button className="oh-mode" disabled={disabled || status?.runtime.motion?.name === "rest"} onClick={() => void act("Go to rest", restOrion)}><Moon size={18} /><span><strong>Go to rest</strong><small>Gently settle down</small></span></button>
          </div>
        </section>
        <button className="oh-talk" onClick={onVoice}><span className="oh-mic"><Mic size={19} /></span><span><strong>Talk with Orion</strong><small>Microphone · {voiceLabel}</small></span><ArrowUpRight className="oh-arrow" size={17} /></button>
      </div>
      <section className="oh-lamp" aria-label="Lamp controls" aria-busy={pending !== null}>
        <header className="oh-panel-heading"><h2>Lamp</h2><button className="oh-switch" role="switch" aria-label="Lamp power" aria-checked={appliedLamp?.enabled ?? false} aria-describedby="lamp-command-state lamp-command-help" disabled={disabled} onClick={() => applyLight(!appliedLamp?.enabled)}><span /></button></header>
        <p id="lamp-command-state">{!connection ? "Connect to control the light" : appliedLamp ? `Last set: ${appliedLamp.enabled ? `${appliedLamp.mood} · On` : "Off"}` : "Not set in this session"}</p>
        <div className="oh-dial">
          <svg viewBox="0 0 216 216" aria-hidden="true"><circle className="oh-dial-track" cx="108" cy="108" r="91" fill="none" strokeWidth="8" strokeLinecap="round" strokeDasharray="429 572" /><circle className="oh-dial-value" cx="108" cy="108" r="91" fill="none" strokeWidth="8" strokeLinecap="round" strokeDasharray={`${429 * brightness / 100} 572`} /></svg>
          <div><output htmlFor="lamp-brightness">{brightness}<small>%</small></output><label htmlFor="lamp-brightness">Brightness</label></div>
        </div>
        <div className="oh-range"><SunDim size={18} /><input id="lamp-brightness" type="range" min="1" max="100" value={brightness} onChange={event => setBrightness(Number(event.target.value))} /><Sun size={18} /></div>
        <fieldset className="oh-light-moods"><legend>Light mood</legend><div>{(["Warm white", "Custom color"] as const).map(value => <button key={value} type="button" aria-pressed={mood === value} onClick={() => setMood(value)}>{value}</button>)}</div></fieldset>
        {mood === "Custom color" && <div className="oh-color"><div><label htmlFor="lamp-color">Choose your color</label><span className="oh-swatch" style={{ background: `hsl(${hue} 75% 65%)` }} aria-hidden="true" /></div><input id="lamp-color" type="range" min="0" max="360" value={hue} aria-valuetext={hueName(hue)} onChange={event => setHue(Number(event.target.value))} /></div>}
        <button className="oh-apply" disabled={disabled} onClick={() => applyLight(true)}>{pending === "Warm white light" || pending === "Custom color" ? "Applying…" : `Apply ${mood.toLowerCase()}`}</button>
        <p className="oh-note" id="lamp-command-help">The switch shows your last lamp command. Character and speech can temporarily take over the light.</p>
      </section>
    </div>
    <section className="oh-expressions" aria-label="Expressions"><header className="oh-panel-heading"><h2>Expressions</h2><button className="oh-text-button" onClick={onCreate}>Make an expression <Plus size={15} /></button></header>
      <div className="oh-expression-list">{[["acknowledge_left", "Acknowledge left"], ["acknowledge_right", "Acknowledge right"]].map(([name, label], index) => <button key={name} disabled={disabled || foregroundBusy} onClick={() => void act(label, value => runScene(value, name))}><span className="oh-expression-icon">{index === 0 ? <MoveUpLeft size={16} /> : <MoveUpRight size={16} />}</span>{label}<Play className="oh-arrow" size={14} /></button>)}</div>
    </section>
    <p className="oh-feedback" role="status"><Info size={14} /><span>{pending ? `${pending}…` : result || (!connection ? "Connect Orion to use character, lamp, and expression controls." : "Connected to Orion.")}</span></p>
  </section>;
}
