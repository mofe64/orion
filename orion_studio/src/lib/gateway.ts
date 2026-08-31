import type { GatewayCapabilities, GatewayStatus } from "../types";

export interface GatewayConnection {
  url: string;
  token: string;
}

export interface PublishedScene {
  api_version: number;
  published: true;
  already_present: boolean;
  name: string;
  relative_path: string;
}

async function request<T>(connection: GatewayConnection, path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${connection.url.replace(/\/$/, "")}${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${connection.token}`,
      "Content-Type": "application/json",
      ...init?.headers,
    },
  });
  const body = await response.json();
  if (!response.ok) throw new Error(body.error?.message ?? `Gateway returned ${response.status}.`);
  return body as T;
}

export function getStatus(connection: GatewayConnection): Promise<GatewayStatus> {
  return request(connection, "/api/v1/status");
}

export function getCapabilities(connection: GatewayConnection): Promise<GatewayCapabilities> {
  return request(connection, "/api/v1/capabilities");
}

export function runScene(connection: GatewayConnection, name: string): Promise<unknown> {
  return request(connection, "/api/v1/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "scene", name }),
  });
}

export function publishScene(
  connection: GatewayConnection,
  document: unknown,
): Promise<PublishedScene> {
  return request(connection, "/api/v1/scenes", {
    method: "POST",
    body: JSON.stringify(document),
  });
}

export function runMotion(connection: GatewayConnection, name: string): Promise<unknown> {
  return request(connection, "/api/v1/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "motion", name }),
  });
}

export function gotoPose(
  connection: GatewayConnection,
  name: string,
  durationSeconds: number,
): Promise<unknown> {
  return request(connection, "/api/v1/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "goto", name, duration_seconds: durationSeconds }),
  });
}

export function cancelRun(
  connection: GatewayConnection,
  kind: "movement" | "scene" | "speech",
  runId: number,
): Promise<unknown> {
  return request(connection, "/api/v1/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "cancel", kind, run_id: runId }),
  });
}
