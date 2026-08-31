import { describe, expect, it } from "vitest";

import { createLightingEffect } from "./lightingEffects";

describe("lighting effect templates", () => {
  it("expands a pulse into an editable rise and return", () => {
    let id = 0;
    const events = createLightingEffect(
      "pulse",
      2,
      { red: 1, green: 2, blue: 3, white: 4 },
      () => `event-${++id}`,
    );
    expect(events).toEqual([
      { id: "event-1", at: 2, type: "light", red: 9, green: 5, blue: 3, white: 52, transition_seconds: 0.12 },
      { id: "event-2", at: 2.12, type: "light", red: 1, green: 2, blue: 3, white: 4, transition_seconds: 0.32 },
    ]);
  });

  it("expands breathe into two runtime-compatible cycles", () => {
    let id = 0;
    const events = createLightingEffect(
      "breathe",
      0.5,
      { red: 0, green: 0, blue: 0, white: 0 },
      () => `event-${++id}`,
    );
    expect(events).toHaveLength(4);
    expect(events.map((event) => event.at)).toEqual([0.5, 1.7, 2.9, 4.1]);
    expect(events.every((event) => event.type === "light")).toBe(true);
    expect(events.at(-1)?.white).toBe(0);
  });
});
