import { request, type GatewayConnection } from "./gateway";

export interface RecorderJoint { temp: number | null; current: number | null; volt_min: number | null; status: number }
export interface RecorderRow {
  t: number; reachable: boolean; joints: Record<string, RecorderJoint>;
  loop: { work_ms: number | null; delay_ms: number | null; late: number | null };
  bus_errors: number | null; cpu_temp: number | null; cpu: number | null; ram: number | null;
}
export interface Incident { t: number; kind: string; severity: "problem" | "warning" | "recovered"; message: string; detail?: string }
export interface ReplySpeed { t: number; session?: string; listen_ms: number | null; think_ms: number | null; first_voice_ms: number; total_ms: number }
export interface RecorderSnapshot { hours: number; row_seconds: number; readings: RecorderRow[]; incidents: Incident[]; latency: ReplySpeed[] }

export interface Initiative {
  situation: { people: number; looking: boolean; focused: boolean; visit: number | null; away_since: number | null; hour: number; busy: boolean;
    lamp: { on: boolean; brightness: number; mood: string }; room_lux: number | null; liveliness: "quiet" | "lively" | "demo"; at: number };
  curiosity: { gestures_used: number; gestures_allowed: number; next_gesture_seconds: number; focused: boolean; curiosity: string; remembered_targets: number };
  quirks: { curiosity: string; chattiness: string };
  lively: { desk_seconds: number | null; dark_seconds: number | null; goodnight_rest_seconds: number | null };
  remark_refusals: Record<string, string | null>;
  /** The owner's Lively rules; missing before 0.1.17. */
  rules?: import("./gateway").LivelyRules;
}

export const recorderRanges = [[1, "Last hour"], [24, "Last day"], [168, "Last 7 days"]] as const;

export function getRecorder(connection: GatewayConnection, hours: number, signal?: AbortSignal): Promise<RecorderSnapshot> {
  return request(connection, `/api/v2/system/recorder?hours=${hours}`, { signal: signal ?? AbortSignal.timeout(15000) });
}

/** Saves everything needed to look into a problem later as one JSON file. */
export async function saveSupportBundle(connection: GatewayConnection): Promise<void> {
  const bundle = await request<unknown>(connection, "/api/v2/system/support-bundle", { signal: AbortSignal.timeout(60000) });
  const url = URL.createObjectURL(new Blob([JSON.stringify(bundle)], { type: "application/json" }));
  const link = document.createElement("a");
  link.href = url; link.download = `orion-support-${new Date().toISOString().replace(/[:.]/g, "-")}.json`; link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export const jointNames: Record<string, string> = {
  base_yaw_joint: "Base", shoulder_pitch_joint: "Shoulder", elbow_pitch_joint: "Elbow", head_pitch_joint: "Head tilt", head_roll_joint: "Neck turn",
};

/** The middle value, for "usually" figures. */
export function median(values: number[]): number | null {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (!sorted.length) return null;
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

const remarkNames: Record<string, string> = {
  person_arrived: "Welcome", person_looking: "Answer a look", new_object: "Something new", light_care: "Offer light",
  break_nudge: "Suggest a break", person_left: "Goodbye", goodnight: "Goodnight",
};
export const remarkName = (event: string) => remarkNames[event] ?? event.replaceAll("_", " ");
