import type { AudioStatus, CompiledTrajectoryPreview, GatewayCapabilities, GatewayStatus } from "../types";
import { MultipartJpegParser, type CameraFrame } from "./cameraStream";

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

export class GatewayError extends Error {
  constructor(message: string, public readonly status: number, public readonly code?: string) { super(message); }
}

async function request<T>(connection: GatewayConnection, path: string, init?: RequestInit): Promise<T> {
  const jsonBody = typeof init?.body === "string";
  const response = await fetch(`${connection.url.replace(/\/$/, "")}${path}`, {
    ...init,
    signal: init?.signal ?? AbortSignal.timeout(5000),
    redirect: "error",
    headers: {
      Authorization: `Bearer ${connection.token}`,
      ...(jsonBody ? { "Content-Type": "application/json" } : {}),
      ...init?.headers,
    },
  });
  const body = await response.json();
  if (!response.ok) throw new GatewayError(body.error?.message ?? `Gateway returned ${response.status}.`, response.status, body.error?.code);
  return body as T;
}

export async function setMasterVolume(connection: GatewayConnection, level: number): Promise<AudioStatus> {
  const response = await request<{result: {audio: AudioStatus}}>(connection, "/api/v2/operations", {
    method: "POST", body: JSON.stringify({operation: "volume", level}),
  });
  return response.result.audio;
}

export interface MusicStatus {
  move_to_music?: DanceMode;
  signed_in: boolean;
  client_id: string;
  name: string;
  redirect_uri: string;
  now_playing: {title: string; artist: string; cover: string | null; playing: boolean} | null;
  error: string | null;
}
export async function getMusic(connection: GatewayConnection, signal?: AbortSignal) {
  try {
    return await request<MusicStatus>(connection, "/api/v2/music", {signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(15000)]) : AbortSignal.timeout(15000), cache:"no-store"});
  } catch (error) {
    if (error instanceof GatewayError && error.status === 404) throw new Error("Update Orion to use music in Studio.");
    throw error;
  }
}
export const musicCommand = (connection: GatewayConnection, value: {action: string; query?: string; kind?: string}) =>
  request(connection, "/api/v2/operations", {method:"POST", signal:AbortSignal.timeout(25000), body:JSON.stringify({operation:"music",request:value})});
export const setMusicMovement = (connection: GatewayConnection, mode: DanceMode) =>
  request(connection, "/api/v2/operations", {method:"POST", body:JSON.stringify({operation:"music_movement",mode})});

export interface CameraObservation {
  id: string | number;
  class: string;
  box: [number, number, number, number]; // Normalised x, y, width, height.
}
export interface CameraDetection {
  class?: "face" | "hand";
  box: [number, number, number, number];
  score: number;
  track_id: number;
  chosen_target: boolean;
  capture_boottime_ns: string;
}
export interface CameraDetectionBatch {
  capture_boottime_ns: string | null;
  age_ms: number | null;
  detections: CameraDetection[];
}
export type CameraStreamState = "connecting" | "live" | "reconnecting" | "off" | "unavailable";
export const setCameraEnabled = (connection: GatewayConnection, camera_enabled: boolean) =>
  request<{ camera_enabled: boolean }>(connection, "/api/v2/camera/enabled", { method: "POST", body: JSON.stringify({ camera_enabled }) });
export type TrackingMode = "off" | "idle" | "faces" | "hands";
export interface TrackingSetting { tracking_mode: TrackingMode; camera_enabled: boolean }
export const getTrackingMode = (connection: GatewayConnection, signal?: AbortSignal) =>
  request<TrackingSetting>(connection, "/api/v2/tracking/mode", { signal, cache: "no-store" });
export const setTrackingMode = (connection: GatewayConnection, tracking_mode: TrackingMode, signal?: AbortSignal) =>
  request<TrackingSetting>(connection, "/api/v2/tracking/mode", { method: "POST", signal, body: JSON.stringify({ tracking_mode }) });
export const getCameraObservations = (connection: GatewayConnection, signal?: AbortSignal) =>
  request<CameraObservation[]>(connection, "/api/v2/camera/observations", { signal });
export const getCameraDetections = (connection: GatewayConnection, signal?: AbortSignal) =>
  request<CameraDetectionBatch>(connection, "/api/v2/camera/detections", { signal, cache: "no-store" });

async function cameraResponse(connection: GatewayConnection, path: string, signal: AbortSignal) {
  const response = await fetch(`${connection.url.replace(/\/$/, "")}/api/v2/camera/${path}`, {
    headers: { Authorization: `Bearer ${connection.token}` }, signal, redirect: "error", cache: "no-store",
  });
  if (!response.ok) {
    const body = await response.json();
    throw new GatewayError(body.error?.message ?? "Camera unavailable.", response.status, body.error?.code);
  }
  return response;
}

export async function getCameraSnapshot(connection: GatewayConnection, signal: AbortSignal = AbortSignal.timeout(5000)): Promise<Blob> {
  return (await cameraResponse(connection, "snapshot", signal)).blob();
}

function cameraRetry(delay: number, signal: AbortSignal): Promise<void> {
  return new Promise(resolve => {
    if (signal.aborted) { resolve(); return; }
    const finish = () => { clearTimeout(timer); signal.removeEventListener("abort", finish); resolve(); };
    const timer = setTimeout(finish, delay);
    signal.addEventListener("abort", finish, { once: true });
  });
}

export async function watchCamera(connection: GatewayConnection, onFrame: (frame: CameraFrame) => void | Promise<void>,
  onState: (state: CameraStreamState, message?: string) => void, signal: AbortSignal, retryDelay = 1000): Promise<void> {
  let reconnecting = false;
  let retryMessage: string | undefined;
  while (!signal.aborted) {
    onState(reconnecting ? "reconnecting" : "connecting", retryMessage);
    const idle = new AbortController();
    const combined = AbortSignal.any([signal, idle.signal]);
    let timer = setTimeout(() => idle.abort(), 8000);
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    try {
      const response = await cameraResponse(connection, "stream", combined);
      if (!response.body) throw new Error("Camera stream has no response body.");
      reader = response.body.getReader();
      const parser = new MultipartJpegParser(response.headers.get("Content-Type") ?? "");
      while (!combined.aborted) {
        const { value, done } = await reader.read();
        if (done) { parser.finish(); break; }
        clearTimeout(timer); timer = setTimeout(() => idle.abort(), 5000);
        const frames = parser.push(value);
        if (frames.length && !combined.aborted) {
          await onFrame(frames[frames.length - 1]);
          if (!combined.aborted) { retryMessage = undefined; onState("live"); }
        }
      }
    } catch (error) {
      if (signal.aborted) return;
      if (error instanceof GatewayError && error.code === "camera_off") { onState("off"); return; }
      if (error instanceof GatewayError && (error.status === 401 || error.status === 403)) {
        onState("unavailable", "Reconnect Orion to view its camera."); return;
      }
      retryMessage = error instanceof Error ? error.message : "Camera connection dropped.";
      onState("reconnecting", retryMessage);
    } finally {
      clearTimeout(timer);
      await reader?.cancel().catch(() => {});
      reader?.releaseLock();
      idle.abort();
    }
    if (!signal.aborted) { reconnecting = true; onState("reconnecting", retryMessage); await cameraRetry(retryDelay, signal); }
  }
}

export interface PairingCodeResult {
  api_version: 2;
  expires_in_seconds: number;
  spoken?: boolean;
  message?: string;
}

async function pairingRequest<T>(url: string, path: string, timeout: number, code?: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(`${url.replace(/\/$/, "")}${path}`, {
    method: "POST",
    signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(timeout)]) : AbortSignal.timeout(timeout),
    redirect: "error",
    ...(code === undefined ? {} : {
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code }),
    }),
  });
  const body = await response.json();
  if (!response.ok) throw new GatewayError(body.error?.message ?? `Gateway returned ${response.status}.`, response.status);
  return body as T;
}

export function requestPairingCode(url: string, signal?: AbortSignal): Promise<PairingCodeResult> {
  return pairingRequest(url, "/api/v2/pair/code", 40000, undefined, signal);
}

export function exchangePairingCode(url: string, code: string): Promise<{ api_version: 2; token: string }> {
  return pairingRequest(url, "/api/v2/pair/token", 5000, code);
}

export function getStatus(connection: GatewayConnection): Promise<GatewayStatus> {
  return request(connection, "/api/v2/status");
}

export function getCapabilities(connection: GatewayConnection): Promise<GatewayCapabilities> {
  return request(connection, "/api/v2/capabilities");
}

export function runScene(connection: GatewayConnection, name: string): Promise<unknown> {
  return request(connection, "/api/v2/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "scene", name }),
  });
}

export function previewScene(connection: GatewayConnection, document: unknown): Promise<unknown> {
  return request(connection, "/api/v2/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "preview_scene", document }),
  });
}

export function compileMotionPreview(
  connection: GatewayConnection,
  motion: string | object,
  startPose: string,
  anchorPose?: string,
  poses?: object,
): Promise<CompiledTrajectoryPreview> {
  return request(connection, "/api/v2/trajectory", {
    method: "POST",
    body: JSON.stringify({
      ...(typeof motion === "string" ? { motion } : { document: motion }),
      start_pose: startPose,
      ...(poses ? { poses } : {}),
      ...(anchorPose ? { anchor_pose: anchorPose } : {}),
    }),
  }).catch(error => {
    if (poses && error instanceof Error && error.message.includes("Trajectory preview accepts")) throw new Error("Update Orion’s Studio gateway to preview scene-specific poses.");
    throw error;
  }) as Promise<CompiledTrajectoryPreview>;
}

export function prepareMovement(connection: GatewayConnection): Promise<unknown> {
  return request(connection, "/api/v2/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "prepare_movement" }),
  });
}

export function releaseMovement(connection: GatewayConnection): Promise<unknown> {
  return request(connection, "/api/v2/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "release_movement" }),
  });
}

export function publishScene(
  connection: GatewayConnection,
  document: unknown,
): Promise<PublishedScene> {
  return request(connection, "/api/v2/scenes", {
    method: "POST",
    body: JSON.stringify(document),
  });
}

export function listUserScenes(connection: GatewayConnection): Promise<UserSceneLibrary> {
  return request(connection, "/api/v2/scenes");
}

export function getUserScene(
  connection: GatewayConnection,
  name: string,
): Promise<UserSceneSource> {
  return request(connection, `/api/v2/scenes/${encodeURIComponent(name)}`);
}

export function deleteUserScene(connection: GatewayConnection, name: string, expectedRevision: string): Promise<{ deleted: boolean }> {
  return request(connection, `/api/v2/scenes/${encodeURIComponent(name)}`, {
    method: "DELETE", body: JSON.stringify({ expected_revision: expectedRevision }),
  });
}

export function updateUserScene(
  connection: GatewayConnection,
  name: string,
  expectedRevision: string,
  document: unknown,
): Promise<UpdatedScene> {
  return request(connection, `/api/v2/scenes/${encodeURIComponent(name)}`, {
    method: "PUT",
    body: JSON.stringify({ expected_revision: expectedRevision, document }),
  });
}

export function listUserPoses(connection: GatewayConnection): Promise<UserAssetLibrary> {
  return request(connection, "/api/v2/poses");
}

export function getUserPose(connection: GatewayConnection, name: string): Promise<UserAssetSource> {
  return request(connection, `/api/v2/poses/${encodeURIComponent(name)}`);
}

export function publishPose(connection: GatewayConnection, document: unknown): Promise<PublishedScene> {
  return request(connection, "/api/v2/poses", {
    method: "POST",
    body: JSON.stringify(document),
  });
}

export function listUserMotions(connection: GatewayConnection): Promise<UserAssetLibrary> {
  return request(connection, "/api/v2/motions");
}

export function getUserMotion(connection: GatewayConnection, name: string): Promise<UserAssetSource> {
  return request(connection, `/api/v2/motions/${encodeURIComponent(name)}`);
}

export function publishMotion(connection: GatewayConnection, document: unknown): Promise<PublishedScene> {
  return request(connection, "/api/v2/motions", {
    method: "POST",
    body: JSON.stringify(document),
  });
}

export function runMotion(connection: GatewayConnection, name: string): Promise<unknown> {
  return request(connection, "/api/v2/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "motion", name }),
  });
}

export function gotoPose(
  connection: GatewayConnection,
  name: string,
  durationSeconds: number,
): Promise<unknown> {
  return request(connection, "/api/v2/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "goto", name, duration_seconds: durationSeconds }),
  });
}

export function cancelRun(
  connection: GatewayConnection,
  kind: "movement" | "scene" | "speech",
  runId: number,
): Promise<unknown> {
  return request(connection, "/api/v2/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "cancel", kind, run_id: runId }),
  });
}

export interface SpeechUploadResult {
  api_version: 2;
  accepted: true;
  studio_voice_request_id: string;
  run_id: number;
  state: "queued" | "playing";
}

export interface SpeechRunStatus {
  first_playback_ms?: number | null;
  elapsed_ms?: number;
  api_version: 2;
  run_id: number;
  state: "queued" | "playing" | "completed" | "cancelled" | "failed";
  error?: string | null;
}

export function uploadSpeech(
  connection: GatewayConnection,
  wav: Uint8Array,
  requestId: string,
): Promise<SpeechUploadResult> {
  return request(connection, "/api/v2/speech", {
    method: "POST",
    headers: {
      "Content-Type": "audio/wav",
      "X-Orion-Voice-Request-ID": requestId,
    },
    body: wav.buffer.slice(wav.byteOffset, wav.byteOffset + wav.byteLength) as ArrayBuffer,
  });
}

export function getSpeechRun(connection: GatewayConnection, runId: number): Promise<SpeechRunStatus> {
  return request(connection, `/api/v2/speech/${runId}`);
}

export function setCharacterMode(connection: GatewayConnection, enabled: boolean): Promise<unknown> {
  return request(connection, "/api/v2/operations", {
    method: "POST",
    body: JSON.stringify({ operation: enabled ? "character_start" : "character_stop" }),
  });
}

export function setCharacterState(
  connection: GatewayConnection,
  state: "neutral" | "listening" | "thinking",
): Promise<unknown> {
  return request(connection, "/api/v2/operations", {
    method: "POST",
    body: JSON.stringify({ operation: "character_state", state }),
  });
}

export function restOrion(connection: GatewayConnection): Promise<unknown> {
  return request(connection, "/api/v2/operations", { method: "POST", body: JSON.stringify({ operation: "rest" }) });
}
export function setLamp(connection: GatewayConnection, rgbw: number[]): Promise<unknown> {
  return request(connection, "/api/v2/operations", { method: "POST", body: JSON.stringify({ operation: "lamp", rgbw }) });
}
export type LampUpdate = {on?:boolean;brightness?:number;colors?:import("../types").LampColor[];effect?:string};
export async function updateLamp(connection:GatewayConnection, settings:LampUpdate):Promise<import("../types").LampStatus> {
  await request(connection,"/api/v2/operations",{method:"POST",body:JSON.stringify({operation:"lamp_effect",settings})});
  const response=await request<{result:{lamp:import("../types").LampStatus}}>(connection,"/api/v2/operations",{method:"POST",body:JSON.stringify({operation:"lamp_status"})});
  return response.result.lamp;
}

export function uploadSpeechChunk(connection: GatewayConnection, wav: Uint8Array, requestId: string, runId?: number, sequence = 0): Promise<SpeechUploadResult> {
  return request(connection, runId === undefined ? "/api/v2/speech/stream" : `/api/v2/speech/${runId}/chunks/${sequence}`, {
    method: "POST", headers: { "Content-Type": "audio/wav", "X-Orion-Voice-Request-ID": requestId },
    body: wav.buffer.slice(wav.byteOffset, wav.byteOffset + wav.byteLength) as ArrayBuffer,
  });
}
export function endSpeechStream(connection: GatewayConnection, runId: number, sequence: number): Promise<unknown> {
  return request(connection, `/api/v2/speech/${runId}/end`, { method: "POST", body: JSON.stringify({ sequence }) });
}

export function getRuntimeLogs(connection: GatewayConnection): Promise<{ lines: string[] }> {
  return request(connection, "/api/v2/debug/logs");
}

export function setUserMode(connection: GatewayConnection, mode: "idle" | "lamp"): Promise<unknown> {
  return updateRoutines(connection, { action: "set_mode", mode });
}
export function setAlertSound(connection: GatewayConnection, kind: "alarm" | "timer", sound: string): Promise<unknown> {
  return updateRoutines(connection, { action: "set_sound", kind, sound });
}
export function updateRoutines(connection: GatewayConnection, action: Record<string, unknown>): Promise<unknown> {
  return request(connection, "/api/v2/operations", { method: "POST", body: JSON.stringify({ operation: "routines", request: action }) });
}

export interface ListItem { id: string; text: string; created: string }
export type ListName = "shopping" | "todo";
export type ListChange = { type: "add"; text: string } | { type: "remove"; expected: ListItem } | { type: "clear"; expected: ListItem[]; confirmed: boolean };
export function agentLists(connection: GatewayConnection, list: ListName, change?: ListChange): Promise<ListItem[]> {
  return request(connection, "/api/v2/voice/request", { method: "POST", body: JSON.stringify({ method: "lists", params: { list, change: change ?? null } }) });
}

export const behaviourKeys = ["object_glances", "mutual_gaze", "arrival_notice", "new_object_peek", "presence_defers_rest", "mind_idle_rhythm", "listen_lean_in", "thinking_beats", "wake_sleep_acts", "posture_drift", "act_sounds", "move_to_music", "proactive_speech", "demo_mode"] as const;
export type BehaviourKey = typeof behaviourKeys[number];
export type DanceMode = "off" | "sometimes" | "always";
export type BehaviourSettings = Record<Exclude<BehaviourKey,"move_to_music">, boolean> & {move_to_music: DanceMode};
export const behaviourDefaults: BehaviourSettings = { object_glances:false, mutual_gaze:false, arrival_notice:false, new_object_peek:false, presence_defers_rest:false, mind_idle_rhythm:true, listen_lean_in:true, thinking_beats:true, wake_sleep_acts:true, posture_drift:false, act_sounds:true, move_to_music:"sometimes", proactive_speech:false, demo_mode:false };
export interface BehaviourResponse { behaviours: BehaviourSettings }
export const getBehaviours = (connection: GatewayConnection, signal?: AbortSignal) =>
  request<BehaviourResponse>(connection, "/api/v2/behaviours", { signal });
export const setBehaviours = (connection: GatewayConnection, changes: Partial<BehaviourSettings>, signal?: AbortSignal) =>
  request<BehaviourResponse>(connection, "/api/v2/behaviours", { method: "POST", body: JSON.stringify(changes), signal });

export type Curiosity = "shy" | "normal" | "nosy";
export type Chattiness = "reserved" | "normal" | "chatty";
export interface Quirks { curiosity: Curiosity; chattiness: Chattiness }
export interface PersonalityResponse { quirks: Quirks }
export const getPersonality = (connection: GatewayConnection, signal?: AbortSignal) =>
  request<PersonalityResponse>(connection, "/api/v2/personality", { signal });
export const setPersonality = (connection: GatewayConnection, quirks: Partial<Quirks>, signal?: AbortSignal) =>
  request<PersonalityResponse>(connection, "/api/v2/personality", { method: "POST", body: JSON.stringify({ quirks }), signal });

export function getSystemMetrics(connection: GatewayConnection, signal?: AbortSignal): Promise<import("./systemMetrics").SystemMetrics> {
  return request(connection, "/api/v2/system/metrics", { signal });
}

export function homeRequest<T>(connection: GatewayConnection, path: string, body?: unknown, signal?: AbortSignal, method?: string): Promise<T> {
  return request(connection, `/api/v2/home/${path}`, {
    method: method ?? (body === undefined ? "GET" : "POST"),
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: signal ?? AbortSignal.timeout(path === "onboarding" ? 120000 : 20000),
  });
}
