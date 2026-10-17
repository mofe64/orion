import { useState, type FormEvent } from "react";
import type { HomeDevices, HomeField, HomeFlow as Flow } from "../lib/home";

function options(value: unknown): { value: string; label: string }[] {
  if (Array.isArray(value)) return value.map(item => Array.isArray(item) ? { value: String(item[0]), label: String(item[1]) } : typeof item === "object" && item !== null && "value" in item ? { value: String(item.value), label: String("label" in item ? item.label : item.value) } : { value: String(item), label: String(item) });
  if (value && typeof value === "object") return Object.entries(value).map(([key, label]) => ({ value: key, label: String(label) }));
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
  const label = field.name.replaceAll("_", " ");
  if (kind === "boolean") return <label className="home-check"><input type="checkbox" checked={value === true} onChange={event => set(event.target.checked)}/>{label}</label>;
  let choices = options(config.options ?? field.options);
  if (kind === "entity") {
    const rawDomain = config.domain ?? (config.filter as { domain?: unknown } | undefined)?.domain;
    const domains = rawDomain ? Array.isArray(rawDomain) ? rawDomain : [rawDomain] : [];
    choices = (devices?.areas.flatMap(area => area.entities) ?? []).filter(entity => !domains.length || domains.includes(entity.domain)).map(entity => ({ value: entity.entity_id, label: `${entity.name} · ${entity.area}` }));
  }
  if (kind === "device") choices = (devices?.devices ?? []).map(device => ({ value: device.id, label: device.name_by_user ?? device.name ?? device.id }));
  if (kind === "area") choices = (devices?.area_registry ?? []).map(area => ({ value: area.area_id, label: area.name }));
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
  return <label>{label}<input type={type} required={field.required} min={config.min != null ? Number(config.min) : field.value_min} max={config.max != null ? Number(config.max) : field.value_max} step={String(config.step ?? (field.type === "integer" ? 1 : "any"))} autoComplete={type === "password" ? "new-password" : "off"} value={String(value ?? "")} onChange={event => set(type === "number" && event.target.value !== "" ? Number(event.target.value) : event.target.value)}/></label>;
}
export function HomeFlowForm({ flow, url, devices, busy, onSubmit, onAbort, onRefresh }: { flow: Flow; url: string; devices: HomeDevices | null; busy: boolean; onSubmit: (data: Record<string, unknown>) => void; onAbort: () => void; onRefresh: () => void }) {
  const schema = flow.data_schema ?? [];
  const [values, setValues] = useState<Record<string, unknown>>(() => Object.fromEntries(schema.flatMap(field => {
    const value = field.default ?? field.description?.suggested_value ?? (fieldKind(field) === "boolean" ? false : undefined);
    return value === undefined ? [] : [[field.name, value]];
  })));
  const unsupported = schema.some(field => fieldKind(field) === "unsupported") || !["form", "menu", "create_entry", "abort"].includes(flow.type);
  const submit = (event: FormEvent) => { event.preventDefault(); onSubmit(Object.fromEntries(Object.entries(values).filter(([, value]) => value !== ""))); };
  return <section className="settings-card home-flow" aria-label="Add device setup"><h2>{flow.title ?? flow.handler ?? "Add device"}</h2>{flow.description && <p>{flow.description}</p>}
    {Object.entries(flow.errors ?? {}).map(([field, error]) => <p role="alert" key={field}>{field.replaceAll("_", " ")}: {error.replaceAll("_", " ")}</p>)}
    {flow.type === "create_entry" ? <p role="status">Device added.</p> : flow.type === "abort" ? <p role="status">{flow.reason?.replaceAll("_", " ") ?? "Setup ended."}</p> : unsupported ? <div><p>This setup needs controls available in Home Assistant.</p><a href={url} target="_blank" rel="noreferrer">Continue in Home Assistant →</a><button className="quiet-button" disabled={busy} onClick={onRefresh}>Check setup</button></div> : flow.type === "menu" ? <div className="home-actions">{flow.menu_options?.map(option => <button className="quiet-button" disabled={busy} key={option} onClick={() => onSubmit({ next_step_id: option })}>{option.replaceAll("_", " ")}</button>)}</div> : <form onSubmit={submit}><fieldset disabled={busy}>{schema.map(field => <FlowField key={field.name} field={field} value={values[field.name]} set={value => setValues(previous => ({ ...previous, [field.name]: value }))} devices={devices}/>)}<button className="primary-button" type="submit">{busy ? "Connecting…" : "Continue"}</button></fieldset></form>}
    <button className="quiet-button" disabled={busy} onClick={onAbort}>{["create_entry", "abort"].includes(flow.type) ? "Close" : "Cancel setup"}</button>
  </section>;
}
