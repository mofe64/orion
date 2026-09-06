import { describe, expect, it } from "vitest";
import { customLight } from "./lightEffects";
describe("custom light preview", () => {
  it("matches constant, pulse, breathe and fade stages", () => {
    const event = { id: "a", colors: ["#000000","#ffffff"], duration: 2, period: 2, intensity: .5 };
    expect(customLight({ ...event, effect: "constant" },1)?.red).toBe(0);
    expect(customLight({ ...event, effect: "pulse" },0)?.red).toBe(127.5);
    expect(customLight({ ...event, effect: "pulse" },1)?.red).toBe(0);
    expect(customLight({ ...event, effect: "breathe" },1)?.red).toBe(127.5);
    expect(customLight({ ...event, effect: "fade" },1)?.red).toBe(63.75);
  });
});
