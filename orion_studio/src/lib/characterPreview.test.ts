import { describe, expect, it } from "vitest";

import { firstSeededIdle } from "./characterPreview";

describe("seeded character preview", () => {
  it("reproduces the same coordinator decision from the same seed and anchor profile", () => {
    expect(firstSeededIdle(42, "attentive")).toEqual(firstSeededIdle(42, "attentive"));
    expect(firstSeededIdle(42, "attentive")).toMatchObject({
      category: "micro",
      clip: "idle_attentive_hold",
    });
  });

  it("keeps scheduled delays in the coordinator contract", () => {
    for (let seed = 1; seed <= 100; seed += 1) {
      const choice = firstSeededIdle(seed, seed % 2 ? "home" : "directional");
      if (choice.category === "micro") expect(choice.dueSeconds).toBeGreaterThanOrEqual(8);
      else expect(choice.dueSeconds).toBeGreaterThanOrEqual(35);
      expect(choice.dueSeconds).toBeLessThanOrEqual(choice.category === "micro" ? 20 : 75);
    }
  });

  it("rejects a non-finite preview seed", () => {
    expect(() => firstSeededIdle(Number.NaN)).toThrow("finite");
  });
});
