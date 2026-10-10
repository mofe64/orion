import { describe, expect, it } from "vitest";
import { alertTime } from "./alertTime";
describe("timer display", () => {
  const now = new Date("2026-10-10T21:08:00").getTime();
  it("combines remaining minutes with local clock time", () => {
    const due = now / 1000 + 540;
    const clock = new Date(due * 1000).toLocaleTimeString("en-GB", { hour: "numeric", minute: "2-digit" });
    expect(alertTime(due, now, "en-GB")).toBe(`in 9 min (${clock})`);
  });
  it("handles due and sub-minute alerts without negative countdowns", () => {
    expect(alertTime(now / 1000 + 10, now)).toMatch(/^in less than a minute \(/);
    expect(alertTime(now / 1000 - 10, now)).toMatch(/^due now \(/);
  });
});
