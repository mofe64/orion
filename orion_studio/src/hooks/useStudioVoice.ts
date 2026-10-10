import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { StudioVoicePipeline, DEFAULT_VOICE_SETTINGS, type StudioVoicePhase, type StudioVoiceSnapshot, type VoiceSettings } from "../lib/studioVoicePipeline";
import { OrionSpeechPlayer, type OrionPlaybackSnapshot } from "../lib/studioSpeaker";
import type { GatewayConnection } from "../lib/gateway";
import { failure, forOwner, type Failure } from "../lib/feedback";
import { useScopedFeedback, useScopedNotice } from "./useScopedNotice";
export const VOICE_PHASE_LABELS: Record<StudioVoicePhase,string> = {
  off: "Off", starting: "Loading voice models", ready: "Listening for Hey Orion", wake_candidate: "Wake phrase detected", confirming_wake: "Confirming Hey Orion", conversation_listening: "You can continue without Hey Orion", command_listening: "Listening for your command", transcribing: "Transcribing locally", thinking: "Orion is thinking", synthesizing: "Creating Orion’s voice", speaking: "Orion is speaking", stopping: "Stopping", error: "Needs attention",
};
export const VOICE_FAILURE_MESSAGES = {
  load: "Studio couldn't load Orion's voice settings. Please try again.",
  save: "Studio couldn't save Orion's voice settings. Please try again.",
  toggle: "Studio couldn't change Orion's listening setting. Please try again.",
};
export const loadVoiceSettings = () => forOwner(() => invoke<VoiceSettings>("load_voice_settings"), VOICE_FAILURE_MESSAGES.load);
export const saveVoiceSettings = (settings: VoiceSettings) => forOwner(() => invoke<VoiceSettings>("save_voice_settings", { settings }), VOICE_FAILURE_MESSAGES.save);
export function useStudioVoice(connection: GatewayConnection | null, page = "home") {
  const desktop = isTauri();
  const [settings,setSettings] = useState<VoiceSettings>(DEFAULT_VOICE_SETTINGS);
  const [loadedFor,setLoadedFor] = useState<GatewayConnection | null | undefined>(undefined);
  const loaded = desktop && loadedFor === connection;
  const currentConnection = useRef(connection);
  currentConnection.current = connection;
  const [saving,setSaving] = useState(false);
  const [toggling,setToggling] = useState(false);
  const [loadError,setLoadError] = useScopedFeedback<Failure | null>("voice-settings", connection);
  const [saveError,setSaveError] = useScopedFeedback<Failure | null>(page, connection);
  const [listeningError,setListeningError] = useScopedFeedback<Failure | null>(page, connection);
  const [notice,showNotice] = useScopedNotice(page, connection);
  const [playback,setPlayback] = useState<OrionPlaybackSnapshot>({ runId: null, state: "idle" });
  const pipeline = useMemo(() => new StudioVoicePipeline({ connection: connection ?? undefined, settings, speaker: new OrionSpeechPlayer(() => connection,setPlayback) }),[connection,settings]);
  const [snapshot,setSnapshot] = useState<StudioVoiceSnapshot>(() => pipeline.current());
  const [models,setModels] = useState<NonNullable<StudioVoiceSnapshot["models"]>>([]);
  useEffect(() => {
    let active = true;
    setLoadedFor(undefined); setLoadError(null);
    if (!desktop) return;
    void loadVoiceSettings().then(value => {
      if (active) { setSettings({ ...DEFAULT_VOICE_SETTINGS,...value }); setLoadedFor(connection); }
    }).catch(error => { if (active) setLoadError(failure(VOICE_FAILURE_MESSAGES.load, error)); });
    return () => { active = false; };
  },[connection,desktop,setLoadError]);
  useEffect(() => { setSnapshot(pipeline.current()); return pipeline.subscribe(setSnapshot); },[pipeline]);
  useEffect(() => { if (snapshot.models?.length) setModels(snapshot.models); },[snapshot.models]);
  useEffect(() => { if (connection && loaded) void pipeline.start(); return () => { void pipeline.stop(); }; },[pipeline,connection,loaded]);
  const save = async (value: VoiceSettings) => {
    setSaving(true); setSaveError(null);
    try {
      if (connection && snapshot.muted === false) await pipeline.setMuted(true);
      const saved = await saveVoiceSettings(value);
      if (currentConnection.current === connection) setSettings({ ...DEFAULT_VOICE_SETTINGS,...saved });
    }
    catch (error) { setSaveError(failure(VOICE_FAILURE_MESSAGES.save, error)); throw new Error(VOICE_FAILURE_MESSAGES.save); }
    finally { setSaving(false); }
  };
  const toggle = async () => {
    if (!connection || toggling || snapshot.muted === undefined) return;
    setToggling(true); setListeningError(null); showNotice("");
    try { await pipeline.setMuted(!snapshot.muted); showNotice(pipeline.current().muted ? "Orion listening is off." : "Orion is listening for Hey Orion."); }
    catch (error) { setListeningError(failure(VOICE_FAILURE_MESSAGES.toggle, error)); }
    finally { setToggling(false); }
  };
  return { settings, loaded, loadError, saveError, listeningError, notice, saving, save, snapshot, playback, models, toggle, toggling,
    listening: snapshot.muted === false, label: !connection ? "Connect Orion to enable listening" : !desktop ? "Open Orion Studio’s desktop app to enable listening" : !loaded ? loadError ? "Voice settings need attention" : "Loading Orion’s voice settings…" : snapshot.muted ? "Listening is off" : snapshot.phase === "thinking" && snapshot.activity ? snapshot.activity : VOICE_PHASE_LABELS[snapshot.phase] };
}
export type StudioVoice = ReturnType<typeof useStudioVoice>;
