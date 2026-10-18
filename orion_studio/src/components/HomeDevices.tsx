import { useEffect, useId, useState } from "react";
import { Blinds, Fan, Lightbulb, LockKeyhole, Power, ShieldCheck, Thermometer, Tv, Volume2 } from "lucide-react";
import { controllable, homeText, stateLabel, supports, type HomeCommand, type HomeDevice, type HomeDevices, type HomeEntity } from "../lib/home";

function RangeControl({ label, name, value, disabled, commit }: { label: string; name: string; value: number; disabled: boolean; commit: (value: number) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const save = () => { if (draft !== value) commit(draft); };
  return <label className="home-range"><span>{label}<span className="home-range-value">{Math.round(draft)}%</span></span><input type="range" min={0} max={100} step={1} value={draft} disabled={disabled} aria-label={`${label} for ${name}`} aria-valuetext={`${Math.round(draft)} percent`} onChange={event => setDraft(Number(event.target.value))} onPointerUp={save} onKeyUp={event => { if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(event.key)) save(); }} onBlur={save}/></label>;
}
function Temperature({ entity, disabled, control }: { entity: HomeEntity; disabled: boolean; control: (command: HomeCommand) => void }) {
  const id = useId();
  const temperature = Number(entity.attributes.temperature ?? entity.attributes.current_temperature ?? 21);
  const [draft, setDraft] = useState(String(temperature));
  useEffect(() => setDraft(String(temperature)), [temperature]);
  return <form className="home-temperature" onSubmit={event => { event.preventDefault(); control({ command: "temperature", temperature: Number(draft) }); }}><label htmlFor={id}>Target temperature</label><div><input id={id} type="number" required min={Number(entity.attributes.min_temp ?? 5)} max={Number(entity.attributes.max_temp ?? 35)} step={Number(entity.attributes.target_temp_step ?? .5)} value={draft} onChange={event => setDraft(event.target.value)} disabled={disabled} aria-label={`Target temperature for ${homeText(entity.name)}`}/><span>{String(entity.attributes.temperature_unit ?? "°C")}</span><button type="submit" className="quiet-button" disabled={disabled || draft === String(temperature)}>Set</button></div></form>;
}
function DeviceControls({ entity, busy, control }: { entity: HomeEntity; busy: boolean; control: (command: HomeCommand) => void }) {
  const { domain, state, attributes: a } = entity;
  if (!controllable(entity)) return null;
  const disabled = busy || ["unavailable", "unknown"].includes(state);
  const isOn = !["off", "standby"].includes(state);
  const dimmable = domain === "light" && (a.brightness != null || (Array.isArray(a.supported_color_modes) && a.supported_color_modes.some(mode => !["onoff", "unknown"].includes(String(mode)))));
  const name = homeText(entity.name);
  return <div className="home-device-controls">
    {["light", "switch", "fan", "media_player"].includes(domain) && <button className={`quiet-button home-power ${isOn ? "is-on" : ""}`} disabled={disabled} onClick={() => control({ command: isOn ? "off" : "on" })} aria-label={`${isOn ? "Turn off" : "Turn on"} ${name}`}><Power size={16} aria-hidden="true"/>{isOn ? "Turn off" : "Turn on"}</button>}
    {dimmable && <RangeControl label="Brightness" name={name} value={Math.round(Number(a.brightness ?? 0) / 255 * 100)} disabled={disabled} commit={brightness => control({ command: "on", brightness })}/>}
    {domain === "climate" && <Temperature entity={entity} disabled={disabled} control={control}/>}
    {domain === "cover" && <div className="home-actions">{[["open", "Open", 1], ["close", "Close", 2], ["stop", "Stop", 8]].map(([command, label, feature]) => (a.supported_features == null || supports(entity, Number(feature))) && <button key={command} className="quiet-button" disabled={disabled || command === "open" && state === "open" || command === "close" && state === "closed"} onClick={() => control({ command: String(command) })} aria-label={`${label} ${name}`}>{label}</button>)}</div>}
    {domain === "media_player" && <>
      {(supports(entity, 16384) || supports(entity, 1)) && <button className="quiet-button" disabled={disabled || (state === "playing" ? !supports(entity, 1) : !supports(entity, 16384))} onClick={() => control({ command: state === "playing" ? "pause" : "play" })} aria-label={`${state === "playing" ? "Pause" : "Play"} ${name}`}>{state === "playing" ? "Pause" : "Play"}</button>}
      {supports(entity, 4) && <RangeControl label="Volume" name={name} value={Number(a.volume_level ?? 0) * 100} disabled={disabled} commit={volume => control({ command: "volume", volume })}/>}
    </>}
  </div>;
}
export function DeviceRow({ device, areas, busy, error, added, control, expose, room }: { device: HomeDevice; areas: HomeDevices["area_registry"]; busy: boolean; error?: string; added?: boolean; control: (id: string, command: HomeCommand) => void; expose: (id: string, enabled: boolean) => void; room: (id: string, area: string | null) => void }) {
  const description = useId();
  const controls = device.entities.filter(controllable);
  const primary = controls[0] ?? device.entities[0];
  const Icon = ({ light: Lightbulb, switch: Power, fan: Fan, climate: Thermometer, cover: Blinds, media_player: Tv, lock: LockKeyhole, alarm_control_panel: ShieldCheck, sensor: Thermometer, binary_sensor: ShieldCheck } as Record<string, typeof Lightbulb>)[primary?.domain] ?? Volume2;
  const name = homeText(device.name);
  return <article className={`home-device ${added ? "home-device-added" : ""}`} id={`home-device-${device.id}`} aria-label={name}>
    <div className="home-device-heading"><Icon size={22} aria-hidden="true"/><div><h3 tabIndex={added ? -1 : undefined} data-added={added || undefined}>{name}{added && <span className="home-new">Just added</span>}</h3><p className="home-device-state">{stateLabel(primary)}</p></div></div>
    <label className="home-room-select"><span className="sr-only">Room for {name}</span><select data-room-device={device.id} aria-label={`Room for ${name}`} disabled={busy} value={device.area_id ?? ""} onChange={event => room(device.id, event.target.value || null)}><option value="">No room yet</option>{areas.map(area => <option value={area.area_id} key={area.area_id}>{homeText(area.name, "Room")}</option>)}</select></label>
    <div className="home-device-body">{controls.length ? controls.map(entity => <div key={entity.entity_id}>{controls.length > 1 && <p className="home-control-name">{homeText(entity.name)}</p>}<DeviceControls entity={entity} busy={busy} control={command => control(entity.entity_id, command)}/></div>) : <p className="settings-help">{["lock", "alarm_control_panel", "cover"].includes(primary.domain) ? "For safety, use the device’s own app to control it." : "Readings update automatically."}</p>}
      <div className="home-readings">{device.entities.filter(entity => entity !== primary && !controllable(entity)).map(entity => <span key={entity.entity_id}>{homeText(entity.name)} <strong>{stateLabel(entity)}</strong></span>)}</div>
      {error && <p role="alert" className="settings-error">{error}</p>}
    </div>
    <div className="home-voice">{controls.length ? <><div><span id={`${description}-label`}>Voice control<span className="sr-only"> for {name}</span></span><p id={description}>Let Orion control this with your voice.</p></div><button type="button" className="studio-switch" role="switch" aria-checked={controls.every(entity => entity.exposed)} aria-labelledby={`${description}-label`} aria-describedby={description} disabled={busy} onClick={() => expose(device.id, !controls.every(entity => entity.exposed))}><span/></button></> : <span className="settings-help">Voice control unavailable</span>}</div>
  </article>;
}
export function HomeRooms({ devices, busy, control, expose, room, errors = {}, added = [] }: { devices: HomeDevices; busy: boolean; control: (id: string, command: HomeCommand) => void; expose: (id: string, enabled: boolean) => void; room: (id: string, area: string | null) => void; errors?: Record<string, string>; added?: string[] }) {
  const rooms = [...new Set(devices.devices.map(device => device.area))].sort((a, b) => a === "No room yet" ? 1 : b === "No room yet" ? -1 : a.localeCompare(b));
  return <div className="home-rooms">{rooms.map(name => { const members = devices.devices.filter(device => device.area === name); return <section className="home-room" key={name}><header><h2>{homeText(name, "No room yet")}</h2><span>{members.length} {members.length === 1 ? "device" : "devices"}</span></header><div className="home-device-list">{members.map(device => <DeviceRow key={device.id} device={device} areas={devices.area_registry} busy={busy} control={control} expose={expose} room={room} error={errors[device.id]} added={added.includes(device.id)}/>)}</div></section>; })}</div>;
}
