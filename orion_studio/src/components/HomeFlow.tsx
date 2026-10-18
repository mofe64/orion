import { useEffect, useRef, useState, type FormEvent } from "react";
import { homeText, setupProblem, type HomeDevices, type HomeField, type HomeFlow as Flow } from "../lib/home";
import { HomeAssistantLink, PasswordInput } from "./HomeInputs";

function options(value: unknown): { value: string; label: string }[] {
  if (Array.isArray(value)) return value.map(item => Array.isArray(item) ? { value: String(item[0]), label: homeText(String(item[1]), "Option") } : typeof item === "object" && item !== null && "value" in item ? { value: String(item.value), label: homeText(String("label" in item ? item.label : item.value), "Option") } : { value: String(item), label: homeText(String(item), "Option") });
  if (value && typeof value === "object") return Object.entries(value).map(([key, label]) => ({ value: key, label: homeText(String(label), "Option") }));
  return [];
}
export function fieldKind(field: HomeField): string {
  if (field.selector) {
    const selector = Object.keys(field.selector)[0];
    return ["text", "number", "boolean", "select", "entity", "device", "area", "time", "date", "datetime", "color_temp", "color_rgb"].includes(selector) ? selector : "unsupported";
  }
  return ({ string: "text", password: "password", integer: "number", float: "number", number: "number", boolean: "boolean", select: "select" } as Record<string, string>)[field.type ?? ""] ?? "unsupported";
}
function FlowField({ field, value, set, devices }: { field: HomeField; value: unknown; set: (value: unknown) => void; devices: HomeDevices | null }) {
  const kind = fieldKind(field);
  const config = field.selector?.[kind] ?? {};
  const label = homeText(field.label ?? field.name, "Device setting");
  if (kind === "boolean") return <label className="home-check"><input type="checkbox" checked={value === true} onChange={event => set(event.target.checked)}/>{label}</label>;
  let choices = options(config.options ?? field.options);
  if (kind === "entity") {
    const rawDomain = config.domain ?? (config.filter as { domain?: unknown } | undefined)?.domain;
    const domains = rawDomain ? Array.isArray(rawDomain) ? rawDomain : [rawDomain] : [];
    choices = (devices?.areas.flatMap(area => area.entities) ?? []).filter(entity => !domains.length || domains.includes(entity.domain)).map(entity => ({ value: entity.entity_id, label: `${homeText(entity.name)} · ${homeText(entity.area, "No room yet")}` }));
  }
  if (kind === "device") choices = (devices?.devices ?? []).map(device => ({ value: device.id, label: homeText(device.name) }));
  if (kind === "area") choices = (devices?.area_registry ?? []).map(area => ({ value: area.area_id, label: homeText(area.name, "Room") }));
  if (["select", "entity", "device", "area"].includes(kind)) {
    const multiple = config.multiple === true;
    if (config.custom_value && !multiple) return <label>{label}<input required={field.required} value={String(value ?? "")} list={`home-${field.name}`} onChange={event => set(event.target.value)}/><datalist id={`home-${field.name}`}>{choices.map(choice => <option key={choice.value} value={choice.value}>{choice.label}</option>)}</datalist></label>;
    return <label>{label}<select required={field.required} multiple={multiple} value={multiple ? (Array.isArray(value) ? value.map(String) : []) : String(value ?? "")} onChange={event => set(multiple ? Array.from(event.target.selectedOptions, option => option.value) : event.target.value)}>{!multiple && <option value="">Choose…</option>}{choices.map(choice => <option key={choice.value} value={choice.value}>{choice.label}</option>)}</select></label>;
  }
  if (kind === "color_rgb") {
    const rgb = Array.isArray(value) ? value : [255, 255, 255];
    return <label>{label}<input type="color" value={`#${rgb.map(part => Number(part).toString(16).padStart(2, "0")).join("")}`} onChange={event => set(event.target.value.slice(1).match(/../g)!.map(part => parseInt(part, 16)))}/></label>;
  }
  const type = kind === "number" || kind === "color_temp" ? "number" : kind === "datetime" ? "datetime-local" : kind === "text" ? String(config.type ?? "text") : kind;
  const props = { required: field.required, min: config.min != null ? Number(config.min) : field.value_min, max: config.max != null ? Number(config.max) : field.value_max, step: String(config.step ?? (field.type === "integer" ? 1 : "any")), value: String(value ?? ""), onChange: (event: React.ChangeEvent<HTMLInputElement>) => set(type === "number" && event.target.value !== "" ? Number(event.target.value) : event.target.value) };
  if (type === "password") return <PasswordInput label={label} autoComplete="current-password" {...props}/>;
  return <label>{label}<input type={type} autoComplete="off" {...props}/></label>;
}
export function HomeFlowForm({ flow, url, devices, busy, onSubmit, onAbort, onRefresh }: { flow: Flow; url: string; devices: HomeDevices | null; busy: boolean; onSubmit: (data: Record<string, unknown>) => void; onAbort: () => void; onRefresh: () => void }) {
  const schema = flow.data_schema ?? [];
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => { heading.current?.focus(); }, []);
  const [values, setValues] = useState<Record<string, unknown>>(() => Object.fromEntries(schema.flatMap(field => {
    const value = field.default ?? field.description?.suggested_value ?? (fieldKind(field) === "boolean" ? false : undefined);
    return value === undefined ? [] : [[field.name, value]];
  })));
  const unsupported = schema.some(field => fieldKind(field) === "unsupported") || !["form", "menu", "create_entry", "abort"].includes(flow.type);
  const submit = (event: FormEvent) => { event.preventDefault(); onSubmit(Object.fromEntries(Object.entries(values).filter(([, value]) => value !== ""))); };
  return <section className="settings-card home-flow" aria-label="Add device setup"><p className="eyebrow">ADD TO YOUR HOME</p><h2 ref={heading} tabIndex={-1}>{homeText(flow.title ?? flow.handler, "Set up your device")}</h2>{flow.description && <p>{homeText(flow.description, "Follow the device’s instructions to connect it.")}</p>}
    {Object.entries(flow.errors ?? {}).map(([field, error]) => <p className="settings-error" role="alert" key={field}>{field !== "base" && `${homeText(schema.find(item => item.name === field)?.label ?? field, "Device setting")}: `}{setupProblem(error,flow)}</p>)}
    {flow.type === "create_entry" ? <p role="status">Device added. Choose a room and try its controls below.</p> : flow.type === "abort" ? <div><p role="alert">{setupProblem(flow.reason,flow)}</p><button className="quiet-button" onClick={onAbort}>Back to your devices</button></div> : unsupported ? <div><p>This step needs Home Assistant. Come back here when you finish.</p><HomeAssistantLink url={url}>Continue in Home Assistant</HomeAssistantLink><button className="quiet-button" disabled={busy} onClick={onRefresh}>Check setup</button></div> : flow.type === "menu" ? <div className="home-actions">{flow.menu_options?.map(option => <button className="quiet-button" disabled={busy} key={option} onClick={() => onSubmit({ next_step_id: option })}>{homeText(option, "Continue setup")}</button>)}</div> : <form onSubmit={submit}><fieldset disabled={busy}>{schema.map(field => <FlowField key={field.name} field={field} value={values[field.name]} set={value => setValues(previous => ({ ...previous, [field.name]: value }))} devices={devices}/>)}<button className="primary-button" type="submit">{busy ? "Connecting…" : "Continue"}</button></fieldset></form>}
    {flow.type !== "abort" && <button className="quiet-button" disabled={busy} onClick={onAbort}>{flow.type === "create_entry" ? "Done" : "Cancel setup"}</button>}
  </section>;
}
