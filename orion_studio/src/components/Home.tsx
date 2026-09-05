import { useState } from "react";
import orionModel from "../assets/orion-home.png";
import { ArrowDownToLine, ArrowUpRight, Lightbulb, Mic, Power } from "lucide-react";
import { restOrion, setCharacterMode, setLamp, runScene, type GatewayConnection } from "../lib/gateway";
import { acceptedRun, type TrackedRun } from "./RunFeedback";
import type { GatewayStatus } from "../types";

interface Props {
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

export function Home({ voiceLabel, connection, status, onConnect, onVoice, onCreate, onRefresh, onNotice, onRun }: Props) {
  const [pending, setPending] = useState<string | null>(null);
  const [color, setColor] = useState("#c0dbff");
  const [brightness, setBrightness] = useState(40);
  const [result, setResult] = useState("");
  const act = async (label: string, work: (value: GatewayConnection) => Promise<unknown>) => {
    if (!connection || pending) return;
    setPending(label); setResult("");
    try { const response = await work(connection); const run = acceptedRun(response, label === "Go to rest" ? "movement" : "scene", label); if (run) onRun(run); await onRefresh(); setResult(`${label} accepted.`); onNotice(`${label} accepted.`); }
    catch (error) { const message = error instanceof Error ? error.message : String(error); setResult(message); onNotice(message); }
    finally { setPending(null); }
  };
  const applyLight = (enabled: boolean) => act(enabled ? "Lamp color" : "Light off", async (value) => {
    const rgb = [1, 3, 5].map(index => enabled ? Math.round(parseInt(color.slice(index, index + 2), 16) * brightness / 100) : 0);
    await setLamp(value, [...rgb, 0]);
  });
  const disabled = !connection || pending !== null;
  const foregroundBusy = !!(status?.scene.active || status?.speech.active || (status?.runtime.motion && !status.runtime.motion.name?.startsWith("idle_")));
  return <section className="owner-home" id="workspace" aria-label="Orion home">
    <div className="home-intro"><p className="eyebrow">Orion Studio</p><h1>Hello, Orion.</h1><p>Talk with Orion, choose an expression,<br className="wide-only" /> or use the lamp.</p>
      <div className="home-status"><span className={`state-orb ${status?.character.enabled ? "alive" : ""}`} /><span>{!connection ? "Orion is disconnected" : status?.character.enabled ? `Character · ${status.character.state.replaceAll("_", " ")}` : "Character is off"}</span></div>
      {!connection && <button className="primary-button" onClick={onConnect}>Connect Orion <ArrowUpRight size={16} /></button>}
      <button className="home-voice" onClick={onVoice}><Mic size={18} /><span>Talk with Orion<small>Microphone · {voiceLabel}</small></span><ArrowUpRight size={17} /></button>
      <figure className="home-model"><img src={orionModel} alt="Orion model in the attentive pose" width="800" height="533" /><figcaption>Attentive pose · model illustration</figcaption></figure>
    </div>
    <div className="home-instrument" aria-label="Everyday controls">
      <header><span className="eyebrow">Everyday</span></header>
      <div className="home-action-row"><div><h2>Character</h2><p>Idle movements and expressive responses.</p></div><button disabled={disabled || status?.character.enabled} onClick={() => void act("Character mode", async value => { await setCharacterMode(value, true); })}><Power size={17} />Enter character mode</button></div>
      <div className="home-action-row"><div><h2>Time to rest</h2><p>Turn character off and gently lower Orion to rest.</p></div><button disabled={disabled || status?.runtime.motion?.name === "rest"} onClick={() => void act("Go to rest", restOrion)}><ArrowDownToLine size={17} />Go to rest</button></div>
      <section className="home-light" aria-label="Lamp controls"><div className="home-action-row"><div><h2>Lamp</h2><p>Choose a color and brightness for the lamp.</p></div><button disabled={disabled} onClick={() => void applyLight(true)}><Lightbulb size={17} />Turn on light</button></div>
        <div className="light-settings"><label>Color<input type="color" value={color} onChange={event => setColor(event.target.value)} /></label><label htmlFor="lamp-brightness">Brightness <output>{brightness}%</output><input id="lamp-brightness" type="range" min="1" max="100" value={brightness} onChange={event => setBrightness(Number(event.target.value))} /></label><div className="light-buttons"><button disabled={disabled} onClick={() => void applyLight(true)}>Apply light</button><button disabled={disabled} onClick={() => void applyLight(false)}>Turn off light</button></div></div><p className="field-help">Character mode restores expressive lighting. Speech and expressions can temporarily take over the light.</p>
      </section>
      <p className="home-feedback" role="status">{pending ? `${pending}…` : result || (!connection ? "Connect Orion to use these controls." : "Connected to Orion.")}</p>
    </div>
    <section className="home-expressions"><div><h2>Expressions</h2></div><div className="expression-choices">{[["acknowledge_left", "Acknowledge left"], ["acknowledge_right", "Acknowledge right"]].map(([name, label]) => <button key={name} disabled={disabled || foregroundBusy} onClick={() => void act(label, value => runScene(value, name))}>{label}<ArrowUpRight size={16} /></button>)}</div><button className="quiet-button" onClick={onCreate}>Make an expression <ArrowUpRight size={16} /></button></section>
  </section>;
}
