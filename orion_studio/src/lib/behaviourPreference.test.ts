import { afterEach, expect, it, vi } from "vitest";
import { BehaviourPreference } from "./behaviourPreference";
import type { BehaviourResponse } from "./gateway";
const connection = { url: "http://orion", token: "secret" };
const off: BehaviourResponse = { behaviours: { move_to_music:"sometimes", mind_idle_rhythm:true, listen_lean_in:true, thinking_beats:true, wake_sleep_acts:true, posture_drift:false, act_sounds:true, object_glances: false, mutual_gaze: false, arrival_notice: false, new_object_peek: false, presence_defers_rest: false, proactive_speech: false, demo_mode: false } };
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
afterEach(() => vi.useRealTimers());
it("optimistically saves one key, reverts on failure, and retains the useful error", async () => {
  vi.useFakeTimers();
  let reject!: (error: Error) => void;
  const save = vi.fn(() => new Promise<BehaviourResponse>((_, fail) => { reject = fail; }));
  const model = new BehaviourPreference(connection, { read: vi.fn().mockResolvedValue(off), save });
  model.start(); await flush();
  const saving = model.choose("object_glances", true);
  expect(model.snapshot().values?.object_glances).toBe(true);
  await model.choose("mutual_gaze", true); expect(save).toHaveBeenCalledTimes(1);
  expect(save).toHaveBeenCalledWith(connection, { object_glances: true }, expect.any(AbortSignal));
  reject(new Error("disk full")); await saving;
  expect(model.snapshot()).toMatchObject({ values: off.behaviours, pending: false, notice: "", error: { detail: "disk full" } });
  await vi.advanceTimersByTimeAsync(2000);
  expect(model.snapshot().error?.detail).toBe("disk full"); model.stop();
});
it("does not replace an acknowledged save with an older poll or a disconnected request", async () => {
  vi.useFakeTimers();
  let oldRead!: (value: BehaviourResponse) => void;
  const read = vi.fn().mockResolvedValueOnce(off).mockImplementation(() => new Promise<BehaviourResponse>(resolve => { oldRead = resolve; }));
  const on = { behaviours: { ...off.behaviours, object_glances: true } };
  const model = new BehaviourPreference(connection, { read, save: vi.fn().mockResolvedValue(on) });
  model.start(); await flush(); await vi.advanceTimersByTimeAsync(2000);
  await model.choose("object_glances", true); oldRead(off); await flush();
  expect(model.snapshot()).toMatchObject({ values: on.behaviours, notice: "Saved on Orion." });
  await vi.advanceTimersByTimeAsync(2000); model.stop(); oldRead(off); await flush();
  expect(model.snapshot().values).toEqual(on.behaviours);
});

it("stops hidden Settings reads and refreshes a cancelled save when visible again", async () => {
  vi.useFakeTimers();
  let finishSave!: (value: BehaviourResponse) => void;
  const read = vi.fn().mockResolvedValue(off);
  const save = vi.fn((_connection: unknown, _values: unknown, _signal?: AbortSignal) => new Promise<BehaviourResponse>(resolve => { finishSave = resolve; }));
  const model = new BehaviourPreference(connection, { read, save });
  model.start(); await flush();
  const saving = model.choose("object_glances", true);
  expect(model.snapshot().pending).toBe(true);
  model.stop();
  expect(read.mock.calls[0][1].aborted).toBe(true);
  expect(save.mock.calls[0][2]!.aborted).toBe(true);
  await vi.advanceTimersByTimeAsync(10_000);
  expect(read).toHaveBeenCalledTimes(1);
  expect(model.snapshot()).toMatchObject({ pending: false, values: null });
  model.start(); await flush();
  finishSave({ behaviours: { ...off.behaviours, object_glances: true } });
  await saving;
  expect(read).toHaveBeenCalledTimes(2);
  expect(model.snapshot()).toMatchObject({ pending: false, values: off.behaviours });
  model.stop();
});

it("reads an older Orion without proactive or demo mode and shows both switches off", async () => {
  const { proactive_speech: _proactive, demo_mode: _demo, ...older } = off.behaviours;
  const model = new BehaviourPreference(connection, { read: vi.fn().mockResolvedValue({ behaviours: older }), save: vi.fn() });
  model.start(); await flush();
  expect(model.snapshot()).toMatchObject({ values: off.behaviours, error: null });
  model.stop();
});

it("turning on demo mode shows the reactions the gateway turned on with it", async () => {
  const on = { behaviours: { ...off.behaviours, demo_mode: true, mutual_gaze: true, arrival_notice: true, new_object_peek: true } };
  const save = vi.fn().mockResolvedValue(on);
  const model = new BehaviourPreference(connection, { read: vi.fn().mockResolvedValue(off), save });
  model.start(); await flush();
  await model.choose("demo_mode", true);
  expect(save).toHaveBeenCalledWith(connection, { demo_mode: true }, expect.any(AbortSignal));
  expect(model.snapshot()).toMatchObject({ values: on.behaviours, notice: "Saved on Orion." });
  model.stop();
});

it("saves a choice that sets two settings in one request and skips unchanged ones", async () => {
  const on = { behaviours: { ...off.behaviours, proactive_speech: true, mutual_gaze: true, arrival_notice: true, new_object_peek: true } };
  const save = vi.fn().mockResolvedValue(on);
  const model = new BehaviourPreference(connection, { read: vi.fn().mockResolvedValue(off), save });
  model.start(); await flush();
  await model.chooseMany({ demo_mode: false, proactive_speech: true });
  expect(save).toHaveBeenCalledWith(connection, { proactive_speech: true }, expect.any(AbortSignal));
  expect(model.snapshot().values).toEqual(on.behaviours);
  model.stop();
});
