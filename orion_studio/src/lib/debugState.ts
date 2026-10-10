import { assetDisplayName } from "./displayName";
const LABELS: Record<string,string> = {
  observe: "Observing · torque off", configured: "Configured · torque off", holding: "Holding position", moving: "Moving",
  off: "Mode paused", starting: "Starting · returning home", home_idle: "Idle at home", pose_idle: "Idle at an anchor pose",
  listening: "Listening", thinking: "Thinking", speaking: "Speaking", foreground_scene: "Playing a requested scene",
  settling: "Returning to an anchor", shutting_down: "Pausing · returning home",
  glance: "Looking around", micro: "Small idle gesture", large: "Larger idle gesture",
  idle: "Idle", buffering: "Preparing audio", playing: "Playing audio", completed: "Finished", failed: "Needs attention",
};
export const debugStateLabel = (token: string) => LABELS[token] ?? assetDisplayName(token);
