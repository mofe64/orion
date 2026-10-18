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
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals(); });

describe("pairing and reconnect", () => {
  it("identifies a credential-store wait during startup and clears it on cancellation", async () => {
    const { controller, store, probe } = setup();
    const pending = deferred<typeof target | null>();
    store.load.mockReturnValueOnce(pending.promise);
    const starting = controller.start();
    await vi.waitFor(() => expect(store.load).toHaveBeenCalled());
    expect(controller.current()).toMatchObject({ phase: "loading", step: "credentials" });
    controller.disconnect();
    expect(controller.current()).toMatchObject({ phase: "disconnected", step: null });
    pending.resolve(target);
    await starting;
    expect(controller.current()).toMatchObject({ phase: "disconnected", step: null });
    expect(probe.status).not.toHaveBeenCalled();
  });
  it("distinguishes code exchange, robot verification and a pending credential save", async () => {
    const { store, probe } = setup(null);
    const listing = deferred<[]>();
    const list = vi.fn(() => listing.promise);
    const controller = new PairingController({ ...store, list }, probe);
    const exchange = deferred<{ api_version: number; token: string }>();
    const verification = deferred<GatewayStatus>();
    const saving = deferred<void>();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: () => exchange.promise }));
    probe.status.mockReturnValueOnce(verification.promise);
    store.save.mockReturnValueOnce(saving.promise);
    const pairing = controller.pairWithCode(target.url, "012345");
    expect(controller.current()).toMatchObject({ phase: "connecting", step: "code" });
    exchange.resolve({ api_version: 2, token: target.token });
    await vi.waitFor(() => expect(probe.status).toHaveBeenCalled());
    expect(controller.current()).toMatchObject({ phase: "connecting", step: "verification" });
    verification.resolve(status);
    await vi.waitFor(() => expect(store.save).toHaveBeenCalled());
    expect(controller.current()).toMatchObject({ phase: "connecting", step: "saving", connection: null });
    saving.resolve();
    await vi.waitFor(() => expect(list).toHaveBeenCalled());
    expect(controller.current()).toMatchObject({ phase: "connecting", step: "saving", connection: null });
    listing.resolve([]);
    expect(await pairing).toBe(true);
    expect(controller.current()).toMatchObject({ phase: "connected", step: null });
  });
  it("exchanges a spoken code and saves the returned token through the existing store", async () => {
    const { controller, store, probe } = setup(null);
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ api_version: 2, token: target.token }) });
    vi.stubGlobal("fetch", fetch);
    expect(await controller.pairWithCode("orion.local", "012345")).toBe(true);
    expect(fetch.mock.calls[0][0]).toBe(`${target.url}/api/v2/pair/token`);
    expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({ code: "012345" });
    expect(probe.status).toHaveBeenCalledWith(target);
    expect(probe.capabilities).toHaveBeenCalledWith(target);
    expect(store.save).toHaveBeenCalledExactlyOnceWith(target);
    expect(controller.current()).toMatchObject({ phase: "connected", connection: target });
  });
  it.each([
    [403, "That code is wrong. 4 tries remain.", "Check the code spoken by Orion. That code is wrong. 4 tries remain."],
    [410, "Ask Orion for a new code.", "That pairing code expired or was already used. Ask Orion for a new code."],
    [429, "pairing_busy", "Orion is busy. Wait 15 seconds, then ask for a new code."],
  ])("shows a friendly pairing-code error for HTTP %s", async (httpStatus, message, expected) => {
    const { controller, store } = setup(null);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: httpStatus, json: async () => ({ error: { message } }) }));
    expect(await controller.pairWithCode(target.url, "111111")).toBe(false);
    expect(controller.current()).toMatchObject({ phase: "error", error: expected });
    expect(store.save).not.toHaveBeenCalled();
  });
  it("normalizes a code request and returns the speech fallback without saving a pairing", async () => {
    const { controller, store } = setup(null);
    await controller.start();
    const result = { api_version: 2, expires_in_seconds: 300, spoken: false, message: "The pairing code is in the gateway log." };
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => result });
    vi.stubGlobal("fetch", fetch);
    expect(await controller.requestCode("orion.local")).toEqual(result);
    expect(fetch.mock.calls[0][0]).toBe(`${target.url}/api/v2/pair/code`);
    expect(store.save).not.toHaveBeenCalled();
    expect(controller.current()).toMatchObject({ phase: "unpaired", error: null });
  });
  it("keeps the active lamp connected and polling while requesting another lamp's code", async () => {
    const { controller, store, probe } = setup();
    await controller.start();
    const pending = deferred<{ api_version: number; expires_in_seconds: number }>();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: () => pending.promise }));
    const requesting = controller.requestCode("other-orion.local");
    expect(controller.current()).toMatchObject({ phase: "connected", connection: target, address: target.url, status, capabilities });
    await vi.advanceTimersByTimeAsync(500);
    expect(probe.status).toHaveBeenCalledTimes(2);
    expect(probe.status).toHaveBeenLastCalledWith(target);
    const result = { api_version: 2, expires_in_seconds: 300 };
    pending.resolve(result);
    expect(await requesting).toEqual(result);
    expect(controller.current()).toMatchObject({ phase: "connected", connection: target, address: target.url, status, capabilities });
    await vi.advanceTimersByTimeAsync(500);
    expect(probe.status).toHaveBeenCalledTimes(3);
    expect(probe.capabilities).toHaveBeenCalledTimes(1);
    expect(store.save).not.toHaveBeenCalled();
  });
  it("reports a failed code request without taking the active lamp offline", async () => {
    const { controller, store, probe } = setup();
    await controller.start();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 429, json: async () => ({ error: { message: "pairing_busy" } }) }));
    expect(await controller.requestCode("other-orion.local")).toBeNull();
    expect(controller.current()).toMatchObject({ phase: "connected", connection: target, address: target.url, status, capabilities,
      error: "Orion is busy. Wait 15 seconds, then ask for a new code." });
    await vi.advanceTimersByTimeAsync(500);
    expect(probe.status).toHaveBeenCalledTimes(2);
    expect(probe.status).toHaveBeenLastCalledWith(target);
    expect(controller.current().phase).toBe("connected");
    expect(store.save).not.toHaveBeenCalled();
  });
  it("shows the rate-limit message when requesting another code too soon", async () => {
    const { controller } = setup(null);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 429, json: async () => ({ error: { message: "pairing_busy" } }) }));
    expect(await controller.requestCode(target.url)).toBeNull();
    expect(controller.current().error).toBe("Orion is busy. Wait 15 seconds, then ask for a new code.");
  });
  it("cancels a code request without disconnecting the active lamp or displaying its late result", async () => {
    const { controller, store } = setup();
    await controller.start();
    const pending = deferred<{ api_version: number; expires_in_seconds: number }>();
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: () => pending.promise });
    vi.stubGlobal("fetch", fetch);
    const requesting = controller.requestCode("other-orion.local");
    controller.cancelCodeRequest();
    expect(fetch.mock.calls[0][1].signal.aborted).toBe(true);
    pending.resolve({ api_version: 2, expires_in_seconds: 300 });
    expect(await requesting).toBeNull();
    expect(controller.current()).toMatchObject({ phase: "connected", connection: target, error: null });
    expect(store.save).not.toHaveBeenCalled();
  });
  it("ignores a late token exchange after forgetting the lamp", async () => {
    const { controller, store } = setup(null);
    const pending = deferred<{ api_version: number; token: string }>();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: () => pending.promise }));
    const pairing = controller.pairWithCode(target.url, "012345");
    await controller.forget();
    pending.resolve({ api_version: 2, token: target.token });
    expect(await pairing).toBe(false);
    expect(store.save).not.toHaveBeenCalled();
    expect(controller.current().phase).toBe("unpaired");
  });
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
    await vi.advanceTimersByTimeAsync(500);
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
  it("verifies and saves a changed address using the existing token", async () => {
    const { controller, store, probe } = setup({ ...target, url: "http://orion.mynet:7447" });
    await controller.start();
    expect(await controller.changeAddress("orion.local")).toBe(true);
    expect(probe.status).toHaveBeenLastCalledWith(target);
    expect(probe.capabilities).toHaveBeenLastCalledWith(target);
    expect(store.save).toHaveBeenCalledExactlyOnceWith(target);
    expect(controller.current()).toMatchObject({ phase: "connected", address: target.url });
  });
  it.each(["network", "credential store"])("preserves the old pairing if an address change fails at the %s", async failure => {
    const { controller, store, probe } = setup();
    await controller.start();
    if (failure === "network") probe.status.mockRejectedValueOnce(new TypeError("offline"));
    else store.save.mockRejectedValueOnce(new Error("Keychain locked"));
    expect(await controller.changeAddress("other-orion.local")).toBe(false);
    expect(controller.current()).toMatchObject({ paired: true, address: target.url });
    if (failure === "network") expect(store.save).not.toHaveBeenCalled();
    await controller.reconnect();
    expect(probe.status).toHaveBeenLastCalledWith(target);
    expect(controller.current()).toMatchObject({ phase: "connected", connection: target });
  });
  it("ignores a late address verification after forgetting the robot", async () => {
    const { controller, store, probe } = setup();
    await controller.start();
    const pending = deferred<GatewayStatus>();
    probe.status.mockReturnValueOnce(pending.promise);
    const changing = controller.changeAddress("other-orion.local");
    await controller.forget();
    pending.resolve(status);
    expect(await changing).toBe(false);
    expect(store.save).not.toHaveBeenCalled();
    expect(controller.current().phase).toBe("unpaired");
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
  it("does not return to connecting when a cancelled startup finishes listing saved lamps", async () => {
    const { store, probe } = setup();
    const pending = deferred<[]>();
    const list = vi.fn(() => pending.promise);
    const controller = new PairingController({ ...store, list }, probe);
    const starting = controller.start();
    await vi.waitFor(() => expect(list).toHaveBeenCalled());
    controller.disconnect();
    pending.resolve([]);
    await starting;
    expect(controller.current().phase).toBe("disconnected");
    expect(probe.status).not.toHaveBeenCalled();
  });
  it.each(["orion.local", "orion.local:7447", "http://orion.local:7447/"])("normalizes %s", (value) => {
    expect(normalizeGatewayUrl(value)).toBe(target.url);
  });
  it.each(["http://user:secret@orion.local", "http://orion.local/api", "http://orion.local?token=x", "file:///tmp/a"])("rejects %s", (value) => {
    expect(() => normalizeGatewayUrl(value)).toThrow();
  });
  it("switches between saved lamps and reconnects to the newly active one", async () => {
    const other = { url: "http://ariadne-robot.local:7447", token: "b".repeat(32) };
    let active = target;
    const store = {
      load: vi.fn(async () => active), save: vi.fn(async () => {}), forget: vi.fn(async () => {}),
      list: vi.fn(async () => [{ url: target.url, active: active === target }, { url: other.url, active: active === other }]),
      select: vi.fn(async (url: string) => { active = url === other.url ? other : target; }),
    };
    const controller = new PairingController(store, { status: vi.fn(async () => status), capabilities: vi.fn(async () => capabilities) });
    await controller.start();
    expect(controller.current().lamps).toEqual([{ url: target.url, active: true }, { url: other.url, active: false }]);
    await controller.switchTo(other.url);
    expect(store.select).toHaveBeenCalledWith(other.url);
    expect(controller.current()).toMatchObject({ phase: "connected", address: other.url, connection: other });
    controller.dispose();
  });
});
