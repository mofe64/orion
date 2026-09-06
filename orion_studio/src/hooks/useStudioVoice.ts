import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import { StudioVoicePipeline, DEFAULT_VOICE_SETTINGS, type StudioVoicePhase, type StudioVoiceSnapshot, type VoiceSettings } from "../lib/studioVoicePipeline";
import { OrionSpeechPlayer, type OrionPlaybackSnapshot } from "../lib/studioSpeaker";
import type { GatewayConnection } from "../lib/gateway";
export const VOICE_PHASE_LABELS: Record<StudioVoicePhase,string> = {
  off: "Off", starting: "Loading voice models", ready: "Listening for Hey Orion", wake_candidate: "Wake phrase detected", confirming_wake: "Confirming Hey Orion", conversation_listening: "You can continue without Hey Orion", command_listening: "Listening for your command", transcribing: "Transcribing locally", thinking: "Orion is thinking", synthesizing: "Creating Orion’s voice", speaking: "Orion is speaking", stopping: "Stopping", error: "Needs attention",
};
export function useStudioVoice(connection: GatewayConnection | null, onNotice: (message: string) => void) {
  const [settings,setSettings] = useState<VoiceSettings>(DEFAULT_VOICE_SETTINGS);
  const [loaded,setLoaded] = useState(false);
  const [saving,setSaving] = useState(false);
  const [toggling,setToggling] = useState(false);
  const [playback,setPlayback] = useState<OrionPlaybackSnapshot>({ runId: null, state: "idle" });
  const pipeline = useMemo(() => new StudioVoicePipeline({ connection: connection ?? undefined, settings, speaker: new OrionSpeechPlayer(() => connection,setPlayback) }),[connection,settings]);
  const [snapshot,setSnapshot] = useState<StudioVoiceSnapshot>(() => pipeline.current());
  const [models,setModels] = useState<NonNullable<StudioVoiceSnapshot["models"]>>([]);
  useEffect(() => { let active = true; void invoke<VoiceSettings>("load_voice_settings").then(value => { if (active) { setSettings({ ...DEFAULT_VOICE_SETTINGS,...value }); setLoaded(true); } }).catch(error => { if (active) onNotice(`Voice settings could not load: ${String(error)}`); }); return () => { active = false; }; },[]);
  useEffect(() => { setSnapshot(pipeline.current()); return pipeline.subscribe(setSnapshot); },[pipeline]);
  useEffect(() => { if (snapshot.models?.length) setModels(snapshot.models); },[snapshot.models]);
  useEffect(() => { if (connection && loaded) void pipeline.start(); return () => { void pipeline.stop(); }; },[pipeline,connection,loaded]);
  const save = async (value: VoiceSettings) => {
    if (connection && snapshot.muted !== true && snapshot.phase !== "error") throw new Error("Turn listening off before changing voice models.");
    setSaving(true);
    try { if (connection && snapshot.muted !== true) await pipeline.setMuted(true); const saved = await invoke<VoiceSettings>("save_voice_settings",{ settings: value }); setSettings({ ...DEFAULT_VOICE_SETTINGS,...saved }); onNotice("Voice settings saved."); }
    finally { setSaving(false); }
  };
  const toggle = async () => {
    if (!connection || toggling || snapshot.muted === undefined) return;
    setToggling(true);
    try { await pipeline.setMuted(!snapshot.muted); onNotice(pipeline.current().muted ? "Orion listening is off." : "Orion is listening for Hey Orion."); }
    catch (error) { onNotice(String(error)); }
    finally { setToggling(false); }
  };
  return { settings, loaded, saving, save, snapshot, playback, models, toggle, toggling,
    listening: snapshot.muted === false, label: !connection ? "Connect Orion to enable listening" : snapshot.muted ? "Listening is off" : snapshot.phase === "thinking" && snapshot.activity ? snapshot.activity : VOICE_PHASE_LABELS[snapshot.phase] };
}
export type StudioVoice = ReturnType<typeof useStudioVoice>;
