import { useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { Bug, SlidersHorizontal, Sparkles, AudioLines, Link2 } from "lucide-react";
import type { StudioVoice } from "../hooks/useStudioVoice";
import type { VoiceSettings } from "../lib/studioVoicePipeline";
import type { StudioPreferences } from "../lib/preferences";
import type { GatewayStatus } from "../types";
import "./Settings.css";
import { AgentProfileSettings } from "./AgentProfileSettings";
import { SoundSettings } from "./SoundSettings";
import { FailureNotice } from "./FailureNotice";
import { useScopedFeedback } from "../hooks/useScopedNotice";
import { assetDisplayName } from "../lib/displayName";
import { orionStateLabel } from "../lib/orionMode";
interface Props {
  voice: StudioVoice; preferences: StudioPreferences; onPreferences: (value: StudioPreferences) => boolean | void;
  theme: "light" | "dark"; onTheme: (theme: "light" | "dark") => boolean | void; status: GatewayStatus | null;
  connected: boolean; onConnect: () => void; onHome: () => void;
  notice?: string;
  connectionScope?: unknown;
  desktop?: boolean;
  onSound: (kind: "alarm" | "timer", sound: string) => Promise<void>;
}
export function canSaveVoiceSettings(voice: StudioVoice, draft: VoiceSettings, provider: string, connected: boolean): boolean {
  const model = voice.models.find(value => value.model === draft.model);
  const validAgent = voice.models.length ? !!model && model.efforts.includes(draft.effort) : !!draft.model.trim() && !!draft.effort.trim();
  return connected && voice.loaded && !voice.saving && JSON.stringify(draft) !== JSON.stringify(voice.settings) && provider === "codex" && validAgent;
}
export function Settings({ voice, preferences, onPreferences, theme, onTheme, status, connected, onConnect, onHome, onSound, notice, connectionScope = connected, desktop = isTauri() }: Props) {
  const [draft,setDraft] = useState(voice.settings);
  const [saved,setSaved] = useScopedFeedback<boolean>("settings", connectionScope);
  const [localSaved,setLocalSaved] = useState("");
  useEffect(() => { setDraft(voice.settings); },[voice.settings]);
  const patch = (value: Partial<typeof draft>) => { setDraft(old => ({ ...old,...value })); setSaved(false); };
  const preference = (key: keyof StudioPreferences, value: boolean) => {
    setLocalSaved(onPreferences({ ...preferences, [key]: value }) === false ? "" : key);
  };
  const model = voice.models.find(value => value.model === draft.model);
  const efforts = model?.efforts ?? [];
  const canSave = desktop && canSaveVoiceSettings(voice, draft, "codex", connected);
  return <section className="settings-page settings-owner" aria-labelledby="settings-title"><header className="settings-heading"><h1 id="settings-title">Settings</h1><p>Make Orion feel at home.</p></header>
    <div className="settings-stack">
      <section className="settings-card"><header><Link2 size={20} /><div><h2>Orion</h2><p>{connected ? "Changes apply immediately on Orion." : "Connect Orion to use its controls."}</p></div></header>
        <div className="settings-row"><div><strong>Orion mode</strong><p>{connected ? orionStateLabel(status) : "Connect Orion to view its mode."}</p></div><button className="quiet-button" type="button" onClick={onHome}>Change mode on Home</button></div>
        <SettingToggle label="Listening" help={voice.label} checked={voice.listening} disabled={!connected || voice.snapshot.muted === undefined || voice.toggling} onChange={() => void voice.toggle()} />
        {voice.notice && <p role="status">{voice.listeningError ? voice.notice : `Saved. ${voice.notice}`}</p>}<FailureNotice value={voice.listeningError} />
        <SoundSettings embedded connected={connected} connectionScope={connectionScope} routines={status?.routines} onSound={onSound} />
        <button className="quiet-button" onClick={onConnect}>Manage connection</button>
      </section>
      <section className="settings-card"><header><Sparkles size={20} /><div><h2>Personality and memory</h2><p>Changes save automatically and start a fresh conversation on Orion’s next request.</p></div></header><AgentProfileSettings onboard desktop={desktop} connected={connected} connectionScope={connectionScope} /></section>
      <section className="settings-card"><header><AudioLines size={20} /><div><h2>Voice</h2><p>Speech recognition and voices run on Orion.</p></div></header>
        {!desktop ? <p className="settings-help">Open Orion Studio’s desktop app and connect Orion to view or change voice settings.</p> : !connected ? <p className="settings-help">Connect Orion to view or change voice settings.</p> : !voice.loaded ? voice.loadError ? <FailureNotice value={voice.loadError} /> : <p role="status">Loading Orion’s voice settings…</p> : <form onSubmit={event => {
          event.preventDefault(); if (!canSave) return;
          void voice.save({ ...draft,provider:"codex" }).then(() => setSaved(true)).catch(() => {});
        }}>
          <p className="settings-model-identity">Provider: Codex</p>
          <div className="settings-voice-fields"><label>Reply model<select value={draft.model} disabled={!voice.models.length || voice.saving} onChange={event => { const next = voice.models.find(item => item.model === event.target.value); if (next) patch({model:next.model,effort:next.efforts.includes(draft.effort) ? draft.effort : next.efforts[0] ?? ""}); }}>{!model && <option value={draft.model}>Saved reply model · saved</option>}{voice.models.map(item => <option key={item.model} value={item.model}>{item.name}</option>)}</select></label>
          <label>Reasoning effort<select disabled={!efforts.length || voice.saving} value={draft.effort} onChange={event => patch({effort:event.target.value})}>{!efforts.includes(draft.effort) && <option value={draft.effort}>{assetDisplayName(draft.effort)} · saved</option>}{efforts.map(effort => <option key={effort} value={effort}>{assetDisplayName(effort)}</option>)}</select></label></div>
          {!voice.models.length && <p className="settings-help">Orion’s model list is unavailable. Your saved model and effort remain selected; you can still save other voice changes.</p>}
          <p className="settings-model-identity">Voice: Alba (British English)</p><p className="settings-help">Voice credit: Alba uses CC BY 4.0 material.</p>
          <p className="settings-help">Uses the Codex sign-in on your Pi. Confirmed command text and retrieved memories are sent to Codex.</p>
          {!connected && <p className="settings-help">Connect Orion to save voice settings.</p>}
          <div className="settings-save"><button className="primary-button" type="submit" disabled={!canSave}>{voice.saving ? "Saving…" : "Save voice settings"}</button><p className="settings-help">Saving turns listening off and restarts the voice coordinator. Turn listening on when Orion is ready.</p></div>
          <FailureNotice value={voice.saveError} />{saved && <p role="status">Saved on Orion. Turn listening on when Orion is ready.</p>}
        </form>}
      </section>
      <section className="settings-card"><header><SlidersHorizontal size={20} /><div><h2>Studio</h2><p>Appearance and preview preferences save automatically on this computer.</p></div></header>
        <label className="settings-row"><span>Appearance {localSaved === "theme" && <small role="status">Saved</small>}</span><select value={theme} onChange={event => setLocalSaved(onTheme(event.target.value as "light" | "dark") === false ? "" : "theme")}><option value="dark">Dark</option><option value="light">Light</option></select></label>
        {notice && <p role="status">{notice}</p>}
        <SettingToggle label="Preview sound" help="Play scene audio in Studio previews." checked={preferences.previewAudio} saved={localSaved === "previewAudio"} onChange={value => preference("previewAudio", value)} />
        <SettingToggle label="Reduce interface motion" help="Minimize interface animations; scene playback stays unchanged." checked={preferences.reduceMotion} saved={localSaved === "reduceMotion"} onChange={value => preference("reduceMotion", value)} />
      </section>
      <section className="settings-card"><header><Bug size={20} /><div><h2>Developer tools</h2><p>Useful details when something needs attention.</p></div></header><SettingToggle label="Enable debug mode" help="Adds Debug to navigation with voice metrics, joint readings, and runtime logs." checked={preferences.debugMode} saved={localSaved === "debugMode"} onChange={value => preference("debugMode",value)} /></section>
    </div>
  </section>;
}
export function SettingToggle({ label,help,checked,disabled,saved,onChange }: { label:string; help?:string; checked:boolean; disabled?:boolean; saved?:boolean; onChange:(value:boolean)=>void }) {
  return <div className="settings-row"><div><strong>{label}</strong>{saved && <small className="settings-saved" role="status">Saved</small>}{help && <p>{help}</p>}</div><button type="button" className="studio-switch" role="switch" aria-label={label} aria-checked={checked} disabled={disabled} onClick={() => onChange(!checked)}><span /></button></div>;
}
