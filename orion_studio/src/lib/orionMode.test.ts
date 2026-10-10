import { describe, expect, it } from "vitest";
import type { GatewayStatus } from "../types";
import { orionModeLabel, orionStateLabel } from "./orionMode";
const status = { character: { enabled: true }, routines: { mode: "lamp" }, rest: { mode: "idle", state: "awake" } } as unknown as GatewayStatus;
describe("owner mode labels", () => {
  it("uses the same runtime mode for Home and Settings without internal state tokens", () => {
    expect(orionModeLabel(status)).toBe("Lamp mode");
    expect(orionStateLabel(status)).toBe("Lamp mode");
    expect(orionStateLabel({ ...status, character: { ...status.character, enabled: false } })).toBe("Lamp mode · paused");
    expect(orionStateLabel({ ...status, routines: undefined })).toBe("Idle mode");
    expect(orionStateLabel(null)).toBe("Awaiting Orion’s status");
  });
  it.each([["resting", "Resting · torque off"], ["going_to_rest", "Moving to rest"], ["waking", "Waking · returning home"], ["fault", "Rest / wake needs attention"]] as const)("preserves the %s safety state", (state, label) => {
    expect(orionStateLabel({ ...status, rest: { ...status.rest!, state } })).toBe(label);
  });
});
