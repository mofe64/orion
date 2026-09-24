import { useState } from "react";
import { AudioLines } from "lucide-react";
import type { StudioVoice } from "../hooks/useStudioVoice";
import type { GatewayStatus } from "../types";
import { VOICE_PRESETS } from "../lib/studioVoicePipeline";

interface Props {
  voice: StudioVoice;
  connected: boolean;
  routines: GatewayStatus["routines"];
  onSound: (kind: "alarm" | "timer", sound: string) => Promise<void>;
}

export function SoundSettings({ voice, connected, routines, onSound }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState("");
  const pocket = voice.settings.ttsModel?.startsWith("pocket-");
  const piper = voice.settings.ttsModel === "piper-alba-medium";
  const sounds = routines?.available_sounds ?? [];
  const available = connected && !!routines?.sounds && sounds.length > 0;
  const save = async (change: () => Promise<unknown>, message: string) => {
    setBusy(true); setError(""); setSaved("");
    try { await change(); setSaved(message); }
    catch (error) { setError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };

  return <section className="settings-card" aria-labelledby="sound-settings-title">
    <header><AudioLines size={20} /><div><h2 id="sound-settings-title">Voice and sounds</h2><p>Choose how Orion speaks and gets your attention.</p></div></header>
    {piper && <p className="settings-help">Orion speaks with Piper Alba Medium. This voice is selected in Speech models.</p>}
    {pocket && <>
      <label>Default Pocket voice<select value={voice.settings.ttsVoice ?? "alba"} disabled={!connected || !voice.loaded || voice.saving || busy} onChange={event => void save(() => voice.save({ ...voice.settings, ttsVoice: event.target.value }), "Default voice saved on Orion.")}>
        {VOICE_PRESETS.map(name => <option key={name} value={name}>{name[0].toUpperCase() + name.slice(1)}</option>)}
      </select></label>
      <p className="settings-help">Saves automatically on Orion and applies to the next reply. You can change voices while Orion is listening or speaking.</p>
    </>}
    {(["alarm", "timer"] as const).map(kind => <label key={kind}>{kind === "alarm" ? "Alarm sound" : "Timer sound"}
      <select value={routines?.sounds?.[kind] ?? ""} disabled={!available || busy} onChange={event => void save(() => onSound(kind, event.target.value), `${kind === "alarm" ? "Alarm" : "Timer"} sound saved on Orion.`)}>
        {!available && <option value="">Unavailable</option>}
        {sounds.map(sound => <option key={sound.id} value={sound.id}>{sound.name}</option>)}
      </select>
    </label>)}
    <p className="settings-help">Two-tone is the default for both. Each choice saves automatically on Orion and applies when the next alert starts, including alerts you have already set. An alert already ringing keeps its sound.</p>
    <p className="settings-help">Sounds repeat until you say “Hey Orion”, stop them in Studio, or five minutes pass. Voice dismissal needs listening to be on.</p>
    {!connected ? <p className="settings-help">Connect Orion to change its voice and sounds.</p> : !available && <p className="settings-help">{routines ? "Update Orion to choose alarm and timer sounds." : "Waiting for Orion’s sound settings…"}</p>}
    {error && <p role="alert" className="settings-error">{error}</p>}
    {saved && <p role="status">{saved}</p>}
  </section>;
}
