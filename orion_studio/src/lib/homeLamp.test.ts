import { describe, expect, it, vi } from "vitest";
import { createLampCommitter, hueName, lampChannels, lampPreview } from "./homeLamp";

describe("Home lamp channel isolation", () => {
  it("uses the physical white channel for warm white, independent of the custom hue", () => {
    expect(lampChannels({ mood: "Warm white", brightness: 40, hue: 280, enabled: true })).toEqual([0, 0, 0, 102]);
    expect(lampChannels({ mood: "Warm white", brightness: 100, hue: 0, enabled: true })).toEqual([0, 0, 0, 255]);
  });
  it("turns every channel off in either mood", () => {
    for (const mood of ["Warm white", "Custom color"] as const) {
      expect(lampChannels({ mood, brightness: 100, hue: 120, enabled: false })).toEqual([0, 0, 0, 0]);
    }
  });
  it("converts the spectrum's primary hues and scales their brightness without adding white", () => {
    const setting = { mood: "Custom color" as const, brightness: 100, enabled: true };
    expect(lampChannels({ ...setting, hue: 0 })).toEqual([233, 99, 99, 0]);
    expect(lampChannels({ ...setting, hue: 120 })).toEqual([99, 233, 99, 0]);
    expect(lampChannels({ ...setting, hue: 240 })).toEqual([99, 99, 233, 0]);
    expect(lampChannels({ ...setting, hue: 360 })).toEqual(lampChannels({ ...setting, hue: 0 }));
    expect(lampChannels({ ...setting, hue: 0, brightness: 40 })).toEqual([93, 40, 40, 0]);
  });
  it("keeps the renderer's preview channels identical to the command", () => {
    expect(lampPreview({ mood: "Warm white", brightness: 40, hue: 210, enabled: true })).toEqual({ red: 0, green: 0, blue: 0, white: 102 });
    expect(hueName(240)).toBe("Blue");
  });
});

describe("lamp release commits", () => {
  it("serializes requests and sends only the newest setting after an in-flight request", async () => {
    vi.useFakeTimers();
    try {
      let finish!: (value: boolean) => void;
      const apply = vi.fn().mockImplementationOnce(() => new Promise<boolean>(resolve => { finish = resolve; })).mockResolvedValue(true);
      const committer = createLampCommitter(apply);
      const setting = { enabled: true, mood: "Warm white" as const, brightness: 40, hue: 0 };
      committer.queue(setting); await vi.advanceTimersByTimeAsync(150);
      committer.queue({ ...setting, brightness: 41 }); await vi.advanceTimersByTimeAsync(150);
      committer.queue({ ...setting, brightness: 42 }); await vi.advanceTimersByTimeAsync(150);
      expect(apply).toHaveBeenCalledTimes(1);
      finish(true); await vi.advanceTimersByTimeAsync(0);
      expect(apply).toHaveBeenCalledTimes(2);
      expect(apply).toHaveBeenLastCalledWith({ ...setting, brightness: 42 });
    } finally { vi.useRealTimers(); }
  });
  it("retains a release while another Home action is busy", async () => {
    vi.useFakeTimers();
    try {
      const apply = vi.fn().mockResolvedValueOnce(false).mockResolvedValue(true);
      const committer = createLampCommitter(apply);
      const setting = { enabled: true, mood: "Warm white" as const, brightness: 44, hue: 0 };
      committer.queue(setting); await vi.advanceTimersByTimeAsync(150);
      expect(apply).toHaveBeenCalledTimes(1);
      await committer.flush();
      expect(apply).toHaveBeenCalledTimes(2);
      expect(apply).toHaveBeenLastCalledWith(setting);
    } finally { vi.useRealTimers(); }
  });
  it("does not miss Home becoming available during the busy handoff", async () => {
    vi.useFakeTimers();
    try {
      let finish!: (value: boolean) => void;
      const apply = vi.fn().mockImplementationOnce(() => new Promise<boolean>(resolve => { finish = resolve; })).mockResolvedValue(true);
      const committer = createLampCommitter(apply);
      const setting = { enabled: true, mood: "Warm white" as const, brightness: 44, hue: 0 };
      committer.queue(setting); await vi.advanceTimersByTimeAsync(150);
      await committer.flush(); finish(false); await vi.advanceTimersByTimeAsync(0);
      expect(apply).toHaveBeenCalledTimes(2);
      expect(apply).toHaveBeenLastCalledWith(setting);
    } finally { vi.useRealTimers(); }
  });
  it("does not drain queued work after cancellation during a request", async () => {
    vi.useFakeTimers();
    try {
      let finish!: (value: boolean) => void;
      const apply = vi.fn(() => new Promise<boolean>(resolve => { finish = resolve; }));
      const committer = createLampCommitter(apply);
      const setting = { enabled: true, mood: "Warm white" as const, brightness: 40, hue: 0 };
      committer.queue(setting); await vi.advanceTimersByTimeAsync(150);
      committer.queue({ ...setting, brightness: 80 }); await vi.advanceTimersByTimeAsync(150);
      committer.cancel(); finish(true); await vi.advanceTimersByTimeAsync(0);
      expect(apply).toHaveBeenCalledTimes(1);
    } finally { vi.useRealTimers(); }
  });
  it("combines rapid releases into the last complete setting", async () => {
    vi.useFakeTimers();
    try {
      const apply = vi.fn(); const committer = createLampCommitter(apply);
      const setting = { enabled: true, mood: "Custom color" as const, brightness: 70, hue: 120 };
      committer.queue({ ...setting, brightness: 50 });
      await vi.advanceTimersByTimeAsync(100);
      committer.queue(setting);
      await vi.advanceTimersByTimeAsync(149);
      expect(apply).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(apply).toHaveBeenCalledExactlyOnceWith(setting);
    } finally { vi.useRealTimers(); }
  });
  it("cancels a release when the page or connection changes", async () => {
    vi.useFakeTimers();
    try {
      const apply = vi.fn(); const committer = createLampCommitter(apply);
      committer.queue({ enabled: true, mood: "Warm white", brightness: 40, hue: 0 });
      committer.cancel(); await vi.runAllTimersAsync();
      expect(apply).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
  });
});
