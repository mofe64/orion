import { Mic, MicOff, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import {
  StudioVoicePipeline,
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

export function VoicePanel({ open, connection, onClose, onNotice }: VoicePanelProps) {
  const [playback, setPlayback] = useState<OrionPlaybackSnapshot>({ runId: null, state: "idle" });
  const pipeline = useMemo(() => new StudioVoicePipeline({
    connection: connection ?? undefined,
    speaker: new OrionSpeechPlayer(() => connection, setPlayback),
  }), [connection]);
  const [snapshot, setSnapshot] = useState<StudioVoiceSnapshot>(() => pipeline.current());

  useEffect(() => pipeline.subscribe(setSnapshot), [pipeline]);
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
    <section className="voice-popover" role="dialog" aria-label="Studio voice setup">
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
