import { useState } from "react";
import { AudioLines } from "lucide-react";
import type { GatewayStatus } from "../types";
import { useScopedNotice } from "../hooks/useScopedNotice";

interface Props {
  embedded?: boolean;
  connected: boolean;
  connectionScope?: unknown;
  routines: GatewayStatus["routines"];
  onSound: (kind: "alarm" | "timer", sound: string) => Promise<void>;
}

export function SoundSettings({ embedded = false, connected, routines, onSound, connectionScope = connected }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useScopedNotice("sounds", connectionScope);
  const [saved, setSaved] = useScopedNotice("sounds", connectionScope);
  const sounds = routines?.available_sounds ?? [];
  const available = connected && !!routines?.sounds && sounds.length > 0;
  const save = async (change: () => Promise<unknown>, message: string) => {
    setBusy(true); setError(""); setSaved("");
    try { await change(); setSaved(message); }
    catch (error) { setError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };

  const Heading = embedded ? "h3" : "h2";
  return <section className={embedded ? "settings-sounds" : "settings-card"} aria-labelledby="sound-settings-title">
    <header>{!embedded && <AudioLines size={20} />}<div><Heading id="sound-settings-title">Alert sounds</Heading><p className="settings-help">Choose how Orion gets your attention.</p></div></header>
    {(["alarm", "timer"] as const).map(kind => <label key={kind}>{kind === "alarm" ? "Alarm sound" : "Timer sound"}
      <select value={routines?.sounds?.[kind] ?? ""} disabled={!available || busy} onChange={event => void save(() => onSound(kind, event.target.value), `${kind === "alarm" ? "Alarm" : "Timer"} sound saved on Orion.`)}>
        {!available && <option value="">Unavailable</option>}
        {sounds.map(sound => <option key={sound.id} value={sound.id}>{sound.name}</option>)}
      </select>
    </label>)}
    <p className="settings-help">Choices save automatically for the next alert; an alert already ringing keeps its sound. Sounds stop after five minutes, or sooner if you stop them in Studio or say “Hey Orion” with listening on.</p>
    {!connected ? <p className="settings-help">Connect Orion to change its alert sounds.</p> : !available && <p className="settings-help">{routines ? "Update Orion to choose alarm and timer sounds." : "Waiting for Orion’s sound settings…"}</p>}
    {error && <p role="alert" className="settings-error">{error}</p>}
    {saved && <p role="status">Saved. {saved}</p>}
  </section>;
}
