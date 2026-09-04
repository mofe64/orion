import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GatewayError } from "./gateway";
import { PairingController, normalizeGatewayUrl } from "./pairing";
import type { GatewayStatus, GatewayCapabilities } from "../types";

const target = { url: "http://orion.local:7447", token: "a".repeat(32) };
const status = { character: { enabled: true } } as GatewayStatus;
const capabilities = { capabilities: {} } as GatewayCapabilities;
function setup(saved: typeof target | null = target) {
  const store = { load: vi.fn(async () => saved), save: vi.fn(async () => {}), forget: vi.fn(async () => {}) };
  const probe = { status: vi.fn(async () => status), capabilities: vi.fn(async () => capabilities) };
  const controller = new PairingController(store, probe);
  return { controller, store, probe };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); });

describe("pairing and reconnect", () => {
  it("keeps browser development connections explicitly temporary", async () => {
    const store = { persistent: false, load: async () => null, save: async () => {}, forget: async () => {} };
    const controller = new PairingController(store, { status: async () => status, capabilities: async () => capabilities });
    await controller.pair(target.url, target.token);
    expect(controller.current()).toMatchObject({ phase: "connected", persistent: false });
    controller.dispose();
    const reopened = new PairingController(store);
    await reopened.start();
    expect(reopened.current()).toMatchObject({ phase: "unpaired", persistent: false });
  });
  it("restores a saved pairing without prompting or rewriting it", async () => {
    const { controller, store, probe } = setup();
    await controller.start();
    expect(controller.current()).toMatchObject({ phase: "connected", connection: target, paired: true });
    await vi.advanceTimersByTimeAsync(1000);
    expect(probe.status).toHaveBeenCalledTimes(2);
    expect(probe.capabilities).toHaveBeenCalledTimes(1);
    expect(store.save).not.toHaveBeenCalled();
  });
  it("validates the robot before saving the first pairing", async () => {
    const { controller, store, probe } = setup(null);
    await controller.start();
    expect(controller.current().phase).toBe("unpaired");
    probe.status.mockRejectedValueOnce(new GatewayError("unauthorized", 401));
    expect(await controller.pair(target.url, target.token)).toBe(false);
    expect(store.save).not.toHaveBeenCalled();
    expect(await controller.pair(target.url, target.token)).toBe(true);
    expect(store.save).toHaveBeenCalledWith(target);
  });
  it("does not claim pairing succeeded if the secure store fails", async () => {
    const { controller, store } = setup(null);
    store.save.mockRejectedValueOnce(new Error("Keychain locked"));
    expect(await controller.pair(target.url, target.token)).toBe(false);
    expect(controller.current()).toMatchObject({ phase: "error", paired: false, connection: null, error: "Keychain locked" });
  });
  it("clears stale status and retries a network outage with capped backoff", async () => {
    const { controller, probe } = setup();
    await controller.start();
    probe.status.mockRejectedValue(new TypeError("offline"));
    await vi.advanceTimersByTimeAsync(1000);
    expect(controller.current()).toMatchObject({ phase: "reconnecting", status: null, capabilities: null, connection: null });
    await vi.advanceTimersByTimeAsync(1000 + 2000 + 4000 + 8000 + 15000);
    expect(probe.status).toHaveBeenCalledTimes(7);
    probe.status.mockResolvedValue(status);
    await vi.advanceTimersByTimeAsync(15000);
    expect(controller.current().phase).toBe("connected");
  });
  it("stops retries when a saved credential is rejected", async () => {
    const { controller, probe, store } = setup();
    probe.status.mockRejectedValue(new GatewayError("unauthorized", 401));
    await controller.start();
    await vi.advanceTimersByTimeAsync(60000);
    expect(controller.current()).toMatchObject({ phase: "auth_required", paired: true });
    expect(probe.status).toHaveBeenCalledTimes(1);
    expect(store.forget).not.toHaveBeenCalled();
  });
  it("disconnect pauses retry, reconnect resumes, and forget removes the credential", async () => {
    const { controller, probe, store } = setup();
    await controller.start();
    controller.disconnect();
    await vi.advanceTimersByTimeAsync(60000);
    expect(probe.status).toHaveBeenCalledTimes(1);
    expect(controller.current()).toMatchObject({ phase: "disconnected", paired: true });
    await controller.reconnect();
    expect(controller.current().phase).toBe("connected");
    await controller.forget();
    expect(store.forget).toHaveBeenCalledOnce();
    expect(controller.current()).toMatchObject({ phase: "unpaired", connection: null, address: null });
  });
  it("ignores a late heartbeat after disconnect", async () => {
    const { controller, probe } = setup();
    await controller.start();
    const pending = deferred<GatewayStatus>();
    probe.status.mockReturnValueOnce(pending.promise);
    await vi.advanceTimersByTimeAsync(1000);
    controller.disconnect(); pending.resolve(status);
    await vi.advanceTimersByTimeAsync(60000);
    expect(controller.current()).toMatchObject({ phase: "disconnected", status: null });
    expect(probe.status).toHaveBeenCalledTimes(2);
  });
  it("forgets after an in-flight save instead of resurrecting its credential", async () => {
    const { controller, store } = setup(null);
    const pending = deferred<void>();
    store.save.mockReturnValueOnce(pending.promise);
    const pairing = controller.pair(target.url, target.token);
    await vi.waitFor(() => expect(store.save).toHaveBeenCalled());
    const forget = controller.forget();
    expect(store.forget).not.toHaveBeenCalled();
    pending.resolve();
    await Promise.all([pairing, forget]);
    expect(store.forget).toHaveBeenCalledOnce();
    expect(controller.current().phase).toBe("unpaired");
  });
  it("ignores an old load after a newer startup (React StrictMode)", async () => {
    const { controller, store } = setup();
    const pending = deferred<typeof target | null>();
    store.load.mockReturnValueOnce(pending.promise);
    const oldStart = controller.start();
    await vi.waitFor(() => expect(store.load).toHaveBeenCalled());
    controller.dispose();
    await controller.start();
    pending.resolve(null); await oldStart;
    expect(controller.current().phase).toBe("connected");
  });
  it.each(["orion.local", "orion.local:7447", "http://orion.local:7447/"])("normalizes %s", (value) => {
    expect(normalizeGatewayUrl(value)).toBe(target.url);
  });
  it.each(["http://user:secret@orion.local", "http://orion.local/api", "http://orion.local?token=x", "file:///tmp/a"])("rejects %s", (value) => {
    expect(() => normalizeGatewayUrl(value)).toThrow();
  });
});
