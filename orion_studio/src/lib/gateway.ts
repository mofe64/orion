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
  revision: string;
  relative_path: string;
}

export interface UserSceneSummary {
  name: string;
  revision: string;
  bytes: number;
  relative_path: string;
}

export interface UserSceneLibrary {
  api_version: number;
  scenes: UserSceneSummary[];
}

export interface UserSceneSource {
  api_version: number;
  name: string;
  revision: string;
  relative_path: string;
  yaml: string;
}

export interface UpdatedScene {
  api_version: number;
  updated: boolean;
  name: string;
  revision: string;
  relative_path: string;
}

export interface UserAssetSummary {
  name: string;
  revision: string;
  bytes: number;
  relative_path: string;
}

export interface UserAssetLibrary {
  api_version: number;
  assets: UserAssetSummary[];
}

export interface UserAssetSource {
  api_version: number;
  name: string;
  revision: string;
  relative_path: string;
  yaml: string;
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

export function previewScene(connection: GatewayConnection, document: unknown): Promise<unknown> {
  return request(connection, "/api/v1/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "preview_scene", document }),
  });
}

export function prepareMovement(connection: GatewayConnection): Promise<unknown> {
  return request(connection, "/api/v1/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "prepare_movement" }),
  });
}

export function releaseMovement(connection: GatewayConnection): Promise<unknown> {
  return request(connection, "/api/v1/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "release_movement" }),
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

export function listUserScenes(connection: GatewayConnection): Promise<UserSceneLibrary> {
  return request(connection, "/api/v1/scenes");
}

export function getUserScene(
  connection: GatewayConnection,
  name: string,
): Promise<UserSceneSource> {
  return request(connection, `/api/v1/scenes/${encodeURIComponent(name)}`);
}

export function updateUserScene(
  connection: GatewayConnection,
  name: string,
  expectedRevision: string,
  document: unknown,
): Promise<UpdatedScene> {
  return request(connection, `/api/v1/scenes/${encodeURIComponent(name)}`, {
    method: "PUT",
    body: JSON.stringify({ expected_revision: expectedRevision, document }),
  });
}

export function listUserPoses(connection: GatewayConnection): Promise<UserAssetLibrary> {
  return request(connection, "/api/v1/poses");
}

export function getUserPose(connection: GatewayConnection, name: string): Promise<UserAssetSource> {
  return request(connection, `/api/v1/poses/${encodeURIComponent(name)}`);
}

export function publishPose(connection: GatewayConnection, document: unknown): Promise<PublishedScene> {
  return request(connection, "/api/v1/poses", {
    method: "POST",
    body: JSON.stringify(document),
  });
}

export function listUserMotions(connection: GatewayConnection): Promise<UserAssetLibrary> {
  return request(connection, "/api/v1/motions");
}

export function getUserMotion(connection: GatewayConnection, name: string): Promise<UserAssetSource> {
  return request(connection, `/api/v1/motions/${encodeURIComponent(name)}`);
}

export function publishMotion(connection: GatewayConnection, document: unknown): Promise<PublishedScene> {
  return request(connection, "/api/v1/motions", {
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
