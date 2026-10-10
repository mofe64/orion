import type { GatewayStatus } from "../types";

export function orionModeLabel(status: GatewayStatus): string {
  return (status.routines?.mode ?? status.rest?.mode ?? "idle") === "lamp" ? "Lamp mode" : "Idle mode";
}
export function orionStateLabel(status: GatewayStatus | null): string {
  if (!status) return "Awaiting Orion’s status";
  switch (status.rest?.state) {
    case "resting": return "Resting · torque off";
    case "going_to_rest": return "Moving to rest";
    case "waking": return "Waking · returning home";
    case "fault": return "Rest / wake needs attention";
  }
  return `${orionModeLabel(status)}${status.character.enabled ? "" : " · paused"}`;
}
