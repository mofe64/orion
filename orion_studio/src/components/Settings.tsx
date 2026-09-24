import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { Bug, SlidersHorizontal, BrainCircuit, AudioLines, Link2 } from "lucide-react";
import type { StudioVoice } from "../hooks/useStudioVoice";
import type { VoiceSettings } from "../lib/studioVoicePipeline";
import type { StudioPreferences } from "../lib/preferences";
import type { GatewayStatus } from "../types";
import "./Settings.css";
import { AgentProfileSettings } from "./AgentProfileSettings";
import { SoundSettings } from "./SoundSettings";
interface Props {
  voice: StudioVoice; preferences: StudioPreferences; onPreferences: (value: StudioPreferences) => void;
  theme: "light" | "dark"; onTheme: (theme: "light" | "dark") => void; status: GatewayStatus | null;
  connected: boolean; onConnect: () => void; onCharacter: (enabled: boolean) => Promise<void>;
  onSound: (kind: "alarm" | "timer", sound: string) => Promise<void>;
}
export function canSaveVoiceSettings(voice: StudioVoice, draft: VoiceSettings, provider: string, connected: boolean): boolean {
  const model = voice.models.find(value => value.model === draft.model);
  const validAgent = voice.models.length ? !!model && model.efforts.includes(draft.effort) : !!draft.model.trim() && !!draft.effort.trim();
  return connected && voice.loaded && !voice.saving && JSON.stringify(draft) !== JSON.stringify(voice.settings) && provider === "codex" && validAgent;
}
export function Settings({ voice, preferences, onPreferences, theme, onTheme, status, connected, onConnect, onCharacter, onSound }: Props) {
  const [locations,setLocations] = useState<{asrPath:string|null;ttsPath:string|null;cachePath:string;hubPath:string;asrIdentity:string|null;ttsIdentity:string|null}|null>(null);
  const [locationError,setLocationError] = useState("");
  const [draft,setDraft] = useState(voice.settings);
  const [provider,setProvider] = useState<"codex" | "api_key">("codex");
  const [error,setError] = useState("");
  const [saved,setSaved] = useState(false);
  const [characterBusy,setCharacterBusy] = useState(false);
  useEffect(() => { setDraft(voice.settings); },[voice.settings]);
  const patch = (value: Partial<typeof draft>) => { setDraft(old => ({ ...old,...value })); setSaved(false); setError(""); };
  useEffect(() => {
    let active = true;
    setLocations(null); setLocationError("");
    void invoke<NonNullable<typeof locations>>("voice_model_locations",{settings:draft}).then(value => { if (active) setLocations(value); }).catch(() => { if (active) setLocationError("Could not read local model locations."); });
    return () => { active = false; };
  },[draft.asrModel,draft.ttsModel,draft.asrPath,draft.ttsPath,draft.cachePath]);
  const onboard = voice.settings.ttsModel === "piper-alba-medium";
  const model = voice.models.find(value => value.model === draft.model);
  const efforts = model?.efforts ?? [];
  const canSave = canSaveVoiceSettings(voice, draft, provider, connected);
  return <section className="settings-page" aria-labelledby="settings-title"><header className="settings-heading"><p className="eyebrow">YOUR STUDIO</p><h1 id="settings-title">Settings</h1><p>Make Orion feel at home.</p></header>
    <div className="settings-columns"><div className="settings-stack">
      <section className="settings-card"><header><SlidersHorizontal size={20} /><div><h2>Studio</h2><p>Appearance and preview preferences save automatically.</p></div></header>
        <label className="settings-row"><span>Appearance</span><select value={theme} onChange={event => onTheme(event.target.value as "light" | "dark")}><option value="dark">Dark</option><option value="light">Light</option></select></label>
        <SettingToggle label="Preview sound" help="Play scene audio in Studio previews." checked={preferences.previewAudio} onChange={value => onPreferences({ ...preferences,previewAudio: value })} />
        <SettingToggle label="Reduce interface motion" help="Minimize interface animations; scene playback stays unchanged." checked={preferences.reduceMotion} onChange={value => onPreferences({ ...preferences,reduceMotion: value })} />
      </section>
      <section className="settings-card"><header><Link2 size={20} /><div><h2>Orion</h2><p>{connected ? "Changes apply immediately on your robot." : "Pair Orion to use robot controls."}</p></div></header>
        <SettingToggle label="Listening" help={voice.label} checked={voice.listening} disabled={!connected || voice.snapshot.muted === undefined || voice.toggling} onChange={() => void voice.toggle()} />
        <SettingToggle label="Character mode" help="Let Orion use its idle expressions and movement." checked={!!status?.character.enabled} disabled={!connected || !status || characterBusy} onChange={async value => { setCharacterBusy(true); try { await onCharacter(value); } catch (error) { setError(String(error)); } finally { setCharacterBusy(false); } }} />
        <button className="quiet-button" onClick={onConnect}>Manage connection</button>
      </section>
      <SoundSettings connected={connected} routines={status?.routines} onSound={onSound} />
      <section className="settings-card"><header><Bug size={20} /><div><h2>Developer tools</h2><p>Useful details when something needs attention.</p></div></header><SettingToggle label="Enable debug mode" help="Adds Debug to navigation with voice metrics, joint readings, and runtime logs." checked={preferences.debugMode} onChange={value => onPreferences({ ...preferences,debugMode:value })} /></section>
      <div className="settings-profile-section"><h2>Personality and memory</h2><AgentProfileSettings onboard={onboard} key={String(onboard)} /></div>
    </div>
    <form className="settings-stack" onSubmit={event => { event.preventDefault(); if (!canSave) return; void voice.save({ ...draft,provider:"codex" }).then(() => setSaved(true)).catch(error => setError(error instanceof Error ? error.message : String(error))); }}>
      <section className="settings-card"><header><BrainCircuit size={20} /><div><h2>AI agent</h2><p>Choose how Orion prepares its replies, then save below.</p></div></header>
        <label>Provider<select value={provider} onChange={event => { setProvider(event.target.value as typeof provider); setSaved(false); }}><option value="codex">Codex</option><option value="api_key">API key · Coming soon</option></select></label>
        {provider === "api_key" ? <div className="settings-unavailable"><p>API-key providers are not implemented yet. These fields cannot be submitted or saved.</p><label>Provider name<input placeholder="Provider" autoComplete="off" /></label><label>API key<input type="password" placeholder="API key" autoComplete="off" /></label><button type="button" disabled>API-key setup unavailable</button></div> : <>
          <label>Reply model<select value={draft.model} disabled={!voice.models.length} onChange={event => { const next = voice.models.find(item => item.model === event.target.value); if (next) patch({model:next.model,effort:next.efforts.includes(draft.effort) ? draft.effort : next.efforts[0] ?? ""}); }}>{!model && <option value={draft.model}>{draft.model} · saved</option>}{voice.models.map(item => <option key={item.model} value={item.model}>{item.name}</option>)}</select></label>
          <label>Reasoning effort<select disabled={!efforts.length} value={draft.effort} onChange={event => patch({effort:event.target.value})}>{!efforts.includes(draft.effort) && <option value={draft.effort}>{draft.effort} · saved</option>}{efforts.map(effort => <option key={effort} value={effort}>{effort}</option>)}</select></label>
          {!voice.models.length && <p className="settings-help">Orion’s model list is unavailable. Your saved model and effort remain selected; you can still save other voice changes.</p>}
          <p className="settings-help">Changes to the reply model and speech model use Save voice settings below. Saving turns listening off while Orion reloads; turn it on when ready.</p><p className="settings-help">Active agent: {voice.snapshot.agentModel ?? "Not loaded"} · {voice.snapshot.agentEffort ?? "—"}</p><p className="settings-help">{onboard ? "Uses the Codex sign-in on your Pi." : "Uses your local Codex sign-in."} Confirmed command text and retrieved memories are sent to Codex.</p>
        </>}
      </section>
      <section className="settings-card"><header><AudioLines size={20} /><div><h2>Speech models</h2><p>{onboard ? "Speech recognition and voices run on Orion." : "Local models and their weight locations on this computer."}</p></div></header>
        {onboard ? <>
          <p className="settings-model-identity">Voice: Piper Alba Medium · British English</p>
          <p className="settings-model-identity">Qwen3 ASR · {draft.asrPath}</p>
          <p className="settings-help">Wake phrase: Rustpotter. Speech boundaries: Silero VAD. Model files are managed on the Pi.</p>
          <p className="settings-help">Voice credit: Piper Alba uses CC BY 4.0 material.</p>
        </> : <>
        <fieldset disabled={provider !== "codex"}><legend>Speech to text</legend><p className="settings-model-identity">{locations?.asrIdentity ?? (draft.asrPath ? "Original model ID unavailable. The selected folder will be loaded directly." : draft.asrModel)}</p><FolderSetting label="Speech-to-text weights" value={draft.asrPath ?? ""} detected={locations?.asrPath} onChange={asrPath => patch({asrPath})} onError={setError}/></fieldset>
        <fieldset disabled={provider !== "codex"}><legend>Text to speech</legend><p className="settings-model-identity">{locations?.ttsIdentity ?? (draft.ttsPath ? "Original model ID unavailable. The selected folder will be loaded directly." : draft.ttsModel)}</p><FolderSetting label="Text-to-speech weights" value={draft.ttsPath ?? ""} detected={locations?.ttsPath} onChange={ttsPath => patch({ttsPath})} onError={setError}/></fieldset>
        <FolderSetting label="Model download cache" value={draft.cachePath ?? ""} detected={locations?.cachePath} onChange={cachePath => patch({cachePath})} onError={setError}/>{locationError && <p role="status">{locationError}</p>}
        <p className="settings-help">Downloaded weights stay on this computer and are loaded into memory when Voice starts. A model ID may check for updates; cached files are reused. A selected folder takes priority over the model ID. Supported weights: Qwen3 ASR for speech recognition and Chatterbox for speech synthesis. A folder’s metadata may identify its architecture without identifying the original model repository. Changing the cache affects future loads; it does not move existing weights. These engines require Apple Silicon.</p>
        </>}
        <p className="settings-help">Save voice settings applies changes in this AI agent and Speech models section. Studio appearance, listening, and alert sounds save as soon as you change them.</p>
        {!connected && <p className="settings-help">Connect Orion to save voice settings.</p>}
        {error && <p role="alert" className="settings-error">{error}</p>}{saved && <p role="status">{onboard ? "Saved on Orion." : "Saved on this computer."}</p>}
        <button className="primary-button" type="submit" disabled={!canSave}>{voice.saving ? "Saving…" : "Save voice settings"}</button>
      </section>
    </form></div>
  </section>;
}
export function SettingToggle({ label,help,checked,disabled,onChange }: { label:string; help?:string; checked:boolean; disabled?:boolean; onChange:(value:boolean)=>void }) {
  return <div className="settings-row"><div><strong>{label}</strong>{help && <p>{help}</p>}</div><button type="button" className="studio-switch" role="switch" aria-label={label} aria-checked={checked} disabled={disabled} onClick={() => onChange(!checked)}><span /></button></div>;
}

function FolderSetting({label,value,detected,onChange,onError}:{label:string;value:string;detected?:string|null;onChange:(value:string)=>void;onError:(message:string)=>void}) {
  const [picking,setPicking] = useState(false);
  const choose = async () => {
    setPicking(true);
    try { const selected = await invoke<string|null>("choose_voice_folder",{initial:value || detected || null}); if (selected !== null) onChange(selected); }
    catch(error) { onError(`Could not open folder picker: ${String(error)}`); }
    finally { setPicking(false); }
  };
  return <div className="settings-folder"><strong>{label}</strong><p className="settings-folder-path">{value || detected || (detected === undefined ? "Finding local folder…" : "Not found in the local cache. It will download when needed.")}</p><div><button type="button" className="quiet-button" disabled={picking} onClick={() => void choose()}>{picking ? "Choosing…" : "Choose folder…"}</button>{value && <button type="button" className="quiet-button" onClick={() => onChange("")}>Use default</button>}</div>{!value && detected && <small>Detected automatically</small>}</div>;
}
