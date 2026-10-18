import { describe, expect, it, vi } from "vitest";
import { createVolumeControl } from "./volume";
import type { AudioStatus } from "../types";

const status = (volume: number, revision = 0, epoch = "one"): AudioStatus => ({volume, muted: volume === 0, revision, epoch, save_error: null});
const tick = async () => { await Promise.resolve(); await Promise.resolve(); };

describe("live volume", () => {
  it("sends during a drag and coalesces pending positions into the final value", async () => {
    const replies: Array<(value: AudioStatus) => void> = [];
    const send = vi.fn(() => new Promise<AudioStatus>(resolve => replies.push(resolve)));
    const control = createVolumeControl(send, status(40));
    control.begin(); control.choose(41); control.choose(45); control.choose(60);
    expect(send).toHaveBeenCalledTimes(1); expect(send).toHaveBeenLastCalledWith(41);
    control.receive(status(41, 1)); expect(control.snapshot().level).toBe(60);
    control.release(); replies.shift()!(status(41, 1)); await tick();
    expect(send).toHaveBeenCalledTimes(2); expect(send).toHaveBeenLastCalledWith(60);
    replies.shift()!(status(60, 2)); await tick(); expect(control.snapshot().level).toBe(60);
    control.receive(status(41, 1)); expect(control.snapshot().level).toBe(60);
  });
  it("follows voice changes and accepts a restarted lamp without accepting an old epoch again", () => {
    const control = createVolumeControl(vi.fn(), status(40, 8));
    control.receive(status(50, 9)); expect(control.snapshot().level).toBe(50);
    control.receive(status(0, 0, "two")); expect(control.snapshot().level).toBe(0);
    control.receive(status(50, 10)); expect(control.snapshot().level).toBe(0);
  });
  it("protects a dragged value from voice changes and reconciles on release", async () => {
    const control = createVolumeControl(async () => status(60, 1), status(40));
    control.begin(); control.choose(60); await tick();
    control.receive(status(70, 2)); expect(control.snapshot().level).toBe(60);
    control.release(); expect(control.snapshot().level).toBe(70);
  });
  it("rolls back failed changes and preserves the error", async () => {
    const control = createVolumeControl(async () => { throw new Error("Network"); }, status(40));
    control.choose(0); await tick(); expect(control.snapshot().level).toBe(40);
    expect(control.snapshot().error).toContain("couldn't change");
  });
  it("does not let a late response change a replacement connection", async () => {
    let reply!: (value: AudioStatus) => void;
    const control = createVolumeControl(() => new Promise(resolve => { reply = resolve; }), status(40));
    control.choose(60); control.cancel(); reply(status(60, 1)); await tick();
    control.receive(status(20, 0, "two")); expect(control.snapshot().level).toBe(20);
  });
});
