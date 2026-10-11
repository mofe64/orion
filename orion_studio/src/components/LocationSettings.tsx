import { useEffect, useRef, useState } from "react";
import { MapPin } from "lucide-react";
import { changeAgentProfile, geocodeLocation, loadAgentProfile, type WeatherSettings, type WeatherLocation } from "../lib/agentProfile";
export function LocationSettings({desktop, connected, connectionScope}:{desktop:boolean;connected:boolean;connectionScope:unknown}) {
  const [settings,setSettings]=useState<WeatherSettings|null>(null); const [query,setQuery]=useState("");
  const [matches,setMatches]=useState<WeatherLocation[]>([]); const [busy,setBusy]=useState(false); const [notice,setNotice]=useState("");
  const generation=useRef(0);
  useEffect(()=>{const version=++generation.current; setSettings(null);setMatches([]);setQuery("");setNotice("");setBusy(false);
    if(desktop && connected) void loadAgentProfile().then(p=>{if(version===generation.current)setSettings(p.weather);}).catch(()=>{if(version===generation.current)setNotice("Could not load location. Reconnect to try again.");});
    return ()=>{generation.current++;};
  },[desktop,connected,connectionScope]);
  const run=async(work:()=>Promise<void>)=>{const version=generation.current;setBusy(true);setNotice("");try {await work();} catch {if(version===generation.current)setNotice("Could not save location. Your draft is preserved; refresh Settings and try again.");}finally{if(version===generation.current)setBusy(false);}};
  const save=async(next:WeatherSettings)=>{const version=generation.current;if(!settings)return;const p=await changeAgentProfile({type:"weather",expected:settings,settings:next});if(version===generation.current){setSettings(p.weather);setMatches([]);setNotice("Saved on Orion.");}};
  return <section className="settings-card"><header><MapPin size={20} /><div><h2>Location</h2><p>Weather uses this location. Open-Meteo receives its coordinates when you ask for a forecast.</p></div></header>
    {!desktop || !connected ? <p className="settings-help">Connect Orion in the desktop app to set its location.</p> : <>
      <p className="settings-help">{settings?.location ? `${settings.location.name} · ${settings.location.timezone}` : "No location saved."}</p>
      <form onSubmit={event=>{event.preventDefault();const version=generation.current;void run(async()=>{const result=await geocodeLocation(query);if(version===generation.current){setMatches(result);if(!result.length)setNotice("No matches. Try a nearby town or city.");}});}}><label>Location<input required maxLength={200} value={query} disabled={busy || !settings} onChange={e=>setQuery(e.target.value)} /></label><button className="quiet-button" disabled={busy || !settings} type="submit">Find location</button></form>
      {matches.map((location,i)=><button key={i} className="quiet-button" disabled={busy} onClick={()=>void run(()=>save({...settings!,location}))}>Use {location.name} · {location.timezone}</button>)}
      <label>Temperature units<select value={settings?.temperature_unit??""} disabled={busy || !settings} onChange={e=>void run(()=>save({...settings!,temperature_unit:(e.target.value||null) as WeatherSettings["temperature_unit"]}))}><option value="">Memory preference, otherwise Celsius</option><option value="celsius">Celsius</option><option value="fahrenheit">Fahrenheit</option></select></label>
    </>}{notice && <p role="status" className="settings-help">{notice}</p>}
  </section>;
}
