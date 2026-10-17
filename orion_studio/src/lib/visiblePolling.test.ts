import { describe, it, expect, vi } from "vitest";
import { visiblePolling } from "./visiblePolling";
describe("visible polling", () => {
  it("pauses hidden pages, resumes immediately and cancels on disposal", async () => {
    vi.useFakeTimers();
    const visibility = new EventTarget() as EventTarget & { hidden: boolean };
    visibility.hidden = false;
    const signals: AbortSignal[] = [];
    const task = vi.fn(async (signal: AbortSignal) => { signals.push(signal); });
    const stop = visiblePolling(task, visibility);
    await vi.advanceTimersByTimeAsync(2000);
    expect(task).toHaveBeenCalledTimes(2);
    visibility.hidden = true; visibility.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(10000);
    expect(task).toHaveBeenCalledTimes(2);
    visibility.hidden = false; visibility.dispatchEvent(new Event("visibilitychange"));
    expect(task).toHaveBeenCalledTimes(3);
    stop(); await vi.advanceTimersByTimeAsync(10000);
    expect(task).toHaveBeenCalledTimes(3);
    expect(signals.at(-1)?.aborted).toBe(true);
    vi.useRealTimers();
  });
});
