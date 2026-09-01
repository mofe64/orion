import { Mic, MicOff, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import {
  StudioVoicePipeline,
  type StudioVoicePhase,
  type StudioVoiceSnapshot,
} from "../lib/studioVoicePipeline";

interface VoicePanelProps {
  open: boolean;
  onClose: () => void;
  onNotice: (notice: string) => void;
  onPhaseChange: (phase: StudioVoicePhase) => void;
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

function meterPercent(levelDbfs: number | null): number {
  if (levelDbfs === null) return 0;
  return Math.max(0, Math.min(100, ((levelDbfs + 60) / 60) * 100));
}

export function VoicePanel({ open, onClose, onNotice, onPhaseChange }: VoicePanelProps) {
  const pipeline = useMemo(() => new StudioVoicePipeline(), []);
  const [snapshot, setSnapshot] = useState<StudioVoiceSnapshot>(() => pipeline.current());

  useEffect(() => pipeline.subscribe(setSnapshot), [pipeline]);
  useEffect(() => onPhaseChange(snapshot.phase), [onPhaseChange, snapshot.phase]);
  useEffect(() => () => { void pipeline.stop(); }, [pipeline]);

  const toggleCapture = async () => {
    if (!["off", "error"].includes(snapshot.phase)) {
      await pipeline.stop();
      onNotice("Studio local voice pipeline stopped.");
      return;
    }
    await pipeline.start();
    const result = pipeline.current();
    onNotice(result.phase === "ready"
      ? `Local speech recognition is ready on ${result.deviceLabel}.`
      : result.error ?? "Studio local voice could not start.");
  };

  if (!open) return null;
  const active = !["off", "error"].includes(snapshot.phase);

  return (
    <section className="voice-popover" role="dialog" aria-label="Studio voice setup">
      <header>
        <div><p className="panel-kicker">LOCAL VOICE</p><h2>Studio microphone</h2></div>
        <button className="icon-button" onClick={onClose} aria-label="Close voice setup"><X size={15} /></button>
      </header>

      <div className={`voice-state ${snapshot.phase}`}>
        {active ? <Mic size={17} /> : <MicOff size={17} />}
        <div><strong>{PHASE_LABELS[snapshot.phase]}</strong><span>{snapshot.deviceLabel ?? (snapshot.phase === "starting" ? "Starting persistent worker…" : "No microphone open")}</span></div>
      </div>

      <div className="voice-meter" role="meter" aria-label="Microphone input level" aria-valuemin={-60} aria-valuemax={0} aria-valuenow={snapshot.levelDbfs ?? -60}>
        <span style={{ width: `${meterPercent(snapshot.levelDbfs)}%` }} />
      </div>
      <div className="voice-metadata">
        <span>{snapshot.sampleRate ? `${Math.round(snapshot.sampleRate / 1000)} kHz native → 16 kHz` : "Awaiting audio"}</span>
        <span>{snapshot.levelDbfs === null ? "— dBFS" : `${snapshot.levelDbfs.toFixed(1)} dBFS`}</span>
      </div>

      {snapshot.error && <p className="voice-error">{snapshot.error}</p>}
      {snapshot.transcript && <p className="voice-transcript"><span>Latest command</span>{snapshot.transcript}</p>}
      {snapshot.response && <p className="voice-transcript"><span>Orion</span>{snapshot.response}</p>}

      <button
        className={active ? "stop-button" : "primary-button"}
        disabled={snapshot.phase === "stopping"}
        onClick={() => { void toggleCapture(); }}
      >
        {active ? <MicOff size={15} /> : <Mic size={15} />}
        {active ? "Stop microphone" : snapshot.phase === "error" ? "Try again" : "Enable microphone"}
      </button>

      <div className="voice-provider-list">
        <div><span>Activation</span><strong>{snapshot.wakeModel ?? "Not loaded"}{snapshot.wakeThreshold === null ? "" : ` · ${snapshot.wakeThreshold.toFixed(3)}`}</strong></div>
        <div><span>Speech to text</span><strong>{snapshot.asrModel ?? "Not loaded"}</strong></div>
        <div><span>Agent</span><strong>{snapshot.agentModel ?? "Not loaded"}</strong></div>
        <div><span>Pre-roll</span><strong>3 seconds · memory only</strong></div>
        <div><span>Text to speech</span><strong>{snapshot.ttsModel ?? "Not loaded"}</strong></div>
      </div>
      <small>Wake detection, transcription, and speech generation run locally. The selected agent provider receives only the confirmed transcript.</small>
    </section>
  );
}
