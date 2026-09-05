import { Mic, MicOff, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import {
  StudioVoicePipeline,
  DEFAULT_VOICE_SETTINGS,
  type VoiceSettings,
  type StudioVoicePhase,
  type StudioVoiceSnapshot,
} from "../lib/studioVoicePipeline";
import { OrionSpeechPlayer, type OrionPlaybackSnapshot } from "../lib/studioSpeaker";
import type { GatewayConnection } from "../lib/gateway";

interface VoicePanelProps {
  open: boolean;
  connection: GatewayConnection | null;
  onClose: () => void;
  onNotice: (notice: string) => void;
  onPhaseChange?: (label: string) => void;
}

const PHASE_LABELS: Record<StudioVoicePhase, string> = {
  off: "Off",
  starting: "Loading local voice models",
  ready: "Listening for Hey Orion",
  wake_candidate: "Wake phrase detected",
  confirming_wake: "Confirming Hey Orion",
  command_listening: "Listening for your command",
  transcribing: "Transcribing locally",
  thinking: "Orion is thinking",
  synthesizing: "Creating Orion's voice",
  speaking: "Orion is speaking",
  stopping: "Stopping",
  error: "Needs attention",
};

export function VoicePanel({ open, connection, onClose, onNotice, onPhaseChange }: VoicePanelProps) {
  const panel = useRef<HTMLElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") closeRef.current(); };
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("keydown", escape); previous?.focus(); };
  }, [open]);
  const [settings, setSettings] = useState<VoiceSettings>(() => {
    try {
      const saved = JSON.parse(localStorage.getItem("orion.voice.settings") ?? "null");
      if (saved && typeof saved.model === "string" && saved.model.trim() && typeof saved.effort === "string") return saved;
    } catch { /* Use explicit defaults if browser storage is unavailable. */ }
    return DEFAULT_VOICE_SETTINGS;
  });
  useEffect(() => { try { localStorage.setItem("orion.voice.settings", JSON.stringify(settings)); } catch { /* Session settings remain usable. */ } }, [settings]);
  const [playback, setPlayback] = useState<OrionPlaybackSnapshot>({ runId: null, state: "idle" });
  const pipeline = useMemo(() => new StudioVoicePipeline({
    connection: connection ?? undefined,
    settings,
    speaker: new OrionSpeechPlayer(() => connection, setPlayback),
  }), [connection, settings]);
  const [snapshot, setSnapshot] = useState<StudioVoiceSnapshot>(() => pipeline.current());
  const [catalog, setCatalog] = useState<NonNullable<StudioVoiceSnapshot["models"]>>([]);
  useEffect(() => { if (snapshot.models?.length) setCatalog(snapshot.models); }, [snapshot.models]);

  useEffect(() => pipeline.subscribe(setSnapshot), [pipeline]);
  useEffect(() => { if (snapshot.phase === "wake_candidate" || snapshot.phase === "starting") setPlayback({ runId: null, state: "idle" }); }, [snapshot.phase]);
  useEffect(() => { onPhaseChange?.(PHASE_LABELS[snapshot.phase]); }, [snapshot.phase, onPhaseChange]);
  useEffect(() => () => { void pipeline.stop(); }, [pipeline]);

  const toggleCapture = async () => {
    if (!["off", "error"].includes(snapshot.phase)) {
      await pipeline.stop();
      onNotice("Orion microphone and Studio processing stopped.");
      return;
    }
    await pipeline.start();
    const result = pipeline.current();
    onNotice(result.phase === "ready"
      ? `Speech recognition is ready for ${result.deviceLabel}.`
      : result.error ?? "Orion voice could not start.");
  };

  if (!open) return null;
  const active = !["off", "error"].includes(snapshot.phase);

  return (
    <section ref={panel} className="voice-popover" role="dialog" aria-label="Studio voice setup">
      <header>
        <div><p className="panel-kicker">ORION VOICE</p><h2>Speak through Orion</h2></div>
        <button className="icon-button" onClick={onClose} aria-label="Close voice setup"><X size={15} /></button>
      </header>

      <div className={`voice-state ${snapshot.phase}`}>
        {active ? <Mic size={17} /> : <MicOff size={17} />}
        <div><strong>{PHASE_LABELS[snapshot.phase]}</strong><span>{snapshot.deviceLabel ?? (snapshot.phase === "starting" ? "Starting persistent worker…" : "No microphone open")}</span></div>
      </div>

      <p className="voice-metadata">16 kHz audio from Orion · local wake detection on the Pi</p>

      {snapshot.error && <p className="voice-error">{snapshot.error}</p>}
      {snapshot.transcript && <p className="voice-transcript"><span>Latest command</span>{snapshot.transcript}</p>}
      {snapshot.response && <p className="voice-transcript"><span>Orion</span>{snapshot.response}</p>}
      {playback.state !== "idle" && <p className="voice-playback"><span>Pi playback</span>{playback.state}{playback.runId ? ` · run ${playback.runId}` : ""}</p>}

      <button
        className={active ? "stop-button" : "primary-button"}
        disabled={snapshot.phase === "stopping" || !connection}
        onClick={() => { void toggleCapture(); }}
      >
        {active ? <MicOff size={15} /> : <Mic size={15} />}
        {active ? "Stop Orion microphone" : !connection ? "Connect Orion first" : snapshot.phase === "error" ? "Try again" : "Enable Orion microphone"}
      </button>

      <fieldset disabled={active} className="voice-settings">
        <legend>Reply model</legend>
        <label>Model<input list="orion-agent-models" value={settings.model} onChange={event => setSettings({ ...settings, model: event.target.value })} /></label>
        <datalist id="orion-agent-models"><option value="gpt-5.6-sol">GPT-5.6 Sol</option>{catalog.filter(model => model.model !== "gpt-5.6-sol").map(model => <option key={model.model} value={model.model}>{model.name}</option>)}</datalist>
        <label>Reasoning effort<select value={settings.effort} onChange={event => setSettings({ ...settings, effort: event.target.value })}>{(catalog.find(model => model.model === settings.model)?.efforts ?? ["low", "medium", "high", "xhigh", "max", "ultra"]).map(effort => <option key={effort} value={effort}>{effort}</option>)}</select></label>
        <small>Applies when you enable the microphone. Startup verifies support; Orion never silently switches models.</small>
      </fieldset>
      <details className="voice-debug"><summary>Debug</summary>
        <p>Runtime: {snapshot.runtime ?? "Resolved when Voice starts"}</p>
        <p>Active reply model: {snapshot.agentModel ?? "Not loaded"} · {snapshot.agentEffort ?? settings.effort}</p>
        <dl>{Object.entries(snapshot.latency ?? {}).map(([name, value]) => <div key={name}><dt>{name}</dt><dd>{Math.round(value)} ms</dd></div>)}
          {playback.uploadMs !== undefined && <div><dt>Chunk upload round trips (sum)</dt><dd>{playback.uploadMs} ms</dd></div>}
          {playback.firstPlaybackMs != null && <div><dt>Pi accepted first chunk → player start</dt><dd>{playback.firstPlaybackMs} ms</dd></div>}
          {playback.elapsedMs !== undefined && <div><dt>Pi run elapsed</dt><dd>{playback.elapsedMs} ms</dd></div>}
        </dl>
        <small>Durations use each process’s monotonic clock. Player start is software timing, not measured speaker output. Stages overlap; do not add these values as end-to-end latency.</small>
      </details>
      <div className="voice-provider-list">
        <div><span>Activation</span><strong>{snapshot.wakeModel ?? "Not loaded"}{snapshot.wakeThreshold === null ? "" : ` · ${snapshot.wakeThreshold.toFixed(3)}`}</strong></div>
        <div><span>Speech to text</span><strong>{snapshot.asrModel ?? "Not loaded"}</strong></div>
        <div><span>Agent</span><strong>{snapshot.agentModel ?? "Not loaded"}</strong></div>
        <div><span>Pre-roll</span><strong>3 seconds · memory only</strong></div>
        <div><span>Text to speech</span><strong>{snapshot.ttsModel ?? "Not loaded"}</strong></div>
      </div>
      <small>Rustpotter and microphone capture run on Orion. Audio travels over the local network without encryption to Studio for Qwen confirmation and Chatterbox synthesis. The configured Codex agent receives confirmed command text. Replies play through Orion.</small>
    </section>
  );
}
