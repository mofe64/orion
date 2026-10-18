import { homeRequest, type GatewayConnection } from "./gateway";
export interface HomeStatus { installed: boolean; running: boolean; version: string | null; onboarded: boolean; connected: boolean; installing?: boolean; install_failed?: boolean }
export interface HomeEntity { entity_id: string; name: string; area: string; area_id?: string | null; device_id?: string; domain: string; state: string; exposed: boolean; attributes: Record<string, unknown> }
export interface HomeDevice { id: string; name: string; area: string; area_id?: string | null; config_entries?: string[]; entities: HomeEntity[] }
export interface HomeDevices { areas: { name: string; entities: HomeEntity[] }[]; devices: HomeDevice[]; area_registry: { area_id: string; name: string }[] }
export interface HomeFlow { type: string; flow_id?: string; handler?: string; step_id?: string; title?: string; description?: string; reason?: string; hidden?: boolean; ignored_entry_id?: string; result?: { entry_id?: string }; data_schema?: HomeField[]; errors?: Record<string, string>; menu_options?: string[]; context?: { source?: string; title_placeholders?: Record<string, string> } }
export interface HomeField { name: string; label?: string; type?: string; required?: boolean; default?: unknown; options?: unknown[] | Record<string, string>; selector?: Record<string, Record<string, unknown> | null>; value_min?: number; value_max?: number; description?: { suggested_value?: unknown } }
export type Integration = { domain: string; name: string };
export type HomeCommand = { command: string; brightness?: number; temperature?: number; volume?: number };
const labels: Record<string, string> = { hue: "Philips Hue", samsungtv: "Samsung TV", apple_tv: "Apple TV", cast: "Google Cast", zha: "Zigbee devices", host: "Device address", username: "Username", password: "Password", api_key: "Access key", access_token: "Access key", client_id: "Account ID", client_secret: "Account secret", scan_interval: "Time between checks", manual_setup: "Enter device details", user: "Set up device", confirm: "Confirm", ssid: "Wi-Fi name", cannot_connect: "Could not connect. Check the device is on and try again.", invalid_auth: "The sign-in details did not work. Check them and try again.", unknown: "Something went wrong. Try again.", already_configured: "This device is already in your home.", not_supported: "This device needs to be set up in Home Assistant.", aborted: "Setup cancelled. You can add this device again." };
export function homeText(value: string | undefined, fallback = "Device"): string {
  if (!value) return fallback;
  if (labels[value]) return labels[value];
  if (/\b[a-z_]+\.[a-z0-9_]+\b|\d{4}-\d\d-\d\dT/.test(value)) return fallback;
  const text = value.replaceAll("_", " ").replace(/\bconfig entr(?:y|ies)\b/gi, "device setup").replace(/\bentities\b/gi, "devices").replace(/\bentity\b/gi, "device").replace(/\bintegrations?\b/gi, "device connection").replace(/\b(?:domain|handler|flow|container)\b/gi, "setup");
  return text.charAt(0).toUpperCase() + text.slice(1);
}
export function integrationList(value: unknown): Integration[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap(item => typeof item === "string" ? [{ domain: item, name: homeText(item) }] : item && typeof item.domain === "string" ? [{ domain: item.domain, name: homeText(item.name ?? item.domain) }] : []);
}
export const homeUrl = (connection: GatewayConnection) => { const url = new URL(connection.url); if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) throw new Error("Use the lamp’s local address."); url.port = "8123"; url.protocol = "http:"; url.pathname = "/"; url.search = ""; url.hash = ""; return url.href; };
export const getHome = (connection: GatewayConnection, signal?: AbortSignal) => homeRequest<HomeStatus>(connection, "status", undefined, signal);
export const controllable = (entity: HomeEntity) => ["light", "switch", "fan", "climate", "cover", "media_player"].includes(entity.domain) && !(entity.domain === "cover" && ["garage", "garage_door", "gate"].includes(String(entity.attributes.device_class)));
export const supports = (entity: HomeEntity, bit: number) => (Number(entity.attributes.supported_features ?? 0) & bit) !== 0;
export function stateLabel(entity: HomeEntity): string {
  const { state, domain, attributes: a } = entity;
  if (state === "unavailable") return "Not responding";
  if (state === "unknown") return "No reading yet";
  if (domain === "climate") return state === "off" ? "Off" : Number.isFinite(Number(a.temperature)) ? `${a.temperature} ${a.temperature_unit ?? "°C"}` : "Heating ready";
  if (domain === "sensor") return state.trim() !== "" && Number.isFinite(Number(state)) ? `${Number(state)}${a.unit_of_measurement ? ` ${a.unit_of_measurement}` : ""}` : "No reading yet";
  if (domain === "binary_sensor") return state === "on" ? (["door", "window", "opening"].includes(String(a.device_class)) ? "Open" : "Detected") : state === "off" ? (["door", "window", "opening"].includes(String(a.device_class)) ? "Closed" : "Clear") : "No reading yet";
  return ({ on: "On", off: "Off", playing: "Playing", paused: "Paused", idle: "Ready", standby: "Standby", buffering: "Loading", open: "Open", closed: "Closed", opening: "Opening", closing: "Closing", stopped: "Stopped", locked: "Locked", unlocked: "Unlocked", locking: "Locking", unlocking: "Unlocking", disarmed: "Off", armed_home: "Armed at home", armed_away: "Armed away", armed_night: "Armed for the night", triggered: "Alarm sounding" } as Record<string, string>)[state] ?? "State unavailable";
}
export function optimisticEntity(entity: HomeEntity, command: HomeCommand): HomeEntity {
  const state = ({ on: "on", off: "off", play: "playing", pause: "paused", open: "opening", close: "closing", stop: "stopped" } as Record<string, string>)[command.command] ?? entity.state;
  return { ...entity, state, attributes: { ...entity.attributes, ...(command.brightness != null ? { brightness: command.brightness * 255 / 100 } : {}), ...(command.temperature != null ? { temperature: command.temperature } : {}), ...(command.volume != null ? { volume_level: command.volume / 100 } : {}) } };
}
