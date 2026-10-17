import { homeRequest, type GatewayConnection } from "./gateway";
export interface HomeStatus { installed: boolean; running: boolean; version: string | null; onboarded: boolean; connected: boolean; installing?: boolean; install_failed?: boolean }
export interface HomeEntity { entity_id: string; name: string; area: string; area_id?: string; device_id?: string; domain: string; state: string; exposed: boolean; attributes: Record<string, unknown> }
export interface HomeDevices { areas: { name: string; entities: HomeEntity[] }[]; devices: { id: string; name?: string; name_by_user?: string }[]; area_registry: { area_id: string; name: string }[] }
export interface HomeFlow { type: string; flow_id?: string; handler?: string; step_id?: string; title?: string; description?: string; reason?: string; data_schema?: HomeField[]; errors?: Record<string, string>; menu_options?: string[]; context?: { source?: string; title_placeholders?: Record<string, string> } }
export interface HomeField { name: string; type?: string; required?: boolean; default?: unknown; options?: unknown[] | Record<string, string>; selector?: Record<string, Record<string, unknown> | null>; value_min?: number; value_max?: number; description?: { suggested_value?: unknown } }
export type Integration = { domain: string; name: string };
export function integrationList(value: unknown): Integration[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap(item => typeof item === "string" ? [{ domain: item, name: item.replaceAll("_", " ") }] : item && typeof item.domain === "string" ? [{ domain: item.domain, name: item.name ?? item.domain }] : []);
}
export const homeUrl = (connection: GatewayConnection) => { const url = new URL(connection.url); url.port = "8123"; url.protocol = "http:"; url.pathname = "/"; url.search = ""; url.hash = ""; return url.href; };
export const getHome = (connection: GatewayConnection, signal?: AbortSignal) => homeRequest<HomeStatus>(connection, "status", undefined, signal);
