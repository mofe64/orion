import { invoke, isTauri } from "@tauri-apps/api/core";
import { GatewayError, getCapabilities, getStatus, type GatewayConnection } from "./gateway";
import type { GatewayCapabilities, GatewayStatus } from "../types";

export interface PairingStore {
  readonly persistent?: boolean;
  load(): Promise<GatewayConnection | null>;
  save(connection: GatewayConnection): Promise<void>;
  forget(): Promise<void>;
}
export const nativePairingStore: PairingStore = {
  persistent: isTauri(),
  load: () => isTauri() ? invoke("load_pairing") : Promise.resolve(null),
  save: (pairing) => isTauri() ? invoke("save_pairing", { pairing })
    : Promise.resolve(),
  forget: () => isTauri() ? invoke("forget_pairing") : Promise.resolve(),
};

export type ConnectionPhase = "loading" | "unpaired" | "connecting" | "connected" | "reconnecting" | "disconnected" | "auth_required" | "error";
export interface ConnectionSnapshot {
  phase: ConnectionPhase;
  persistent: boolean;
  paired: boolean;
  address: string | null;
  connection: GatewayConnection | null;
  status: GatewayStatus | null;
  capabilities: GatewayCapabilities | null;
  error: string | null;
}
const INITIAL: ConnectionSnapshot = { phase: "loading", persistent: true, paired: false, address: null,
  connection: null, status: null, capabilities: null, error: null };

export function normalizeGatewayUrl(value: string): string {
  const hasScheme = value.includes("://");
  const url = new URL(hasScheme ? value.trim() : `http://${value.trim()}`);
  if (!hasScheme && !url.port) url.port = "7447";
  if (!["http:", "https:"].includes(url.protocol) || !url.hostname || url.username
      || url.password || url.search || url.hash || url.pathname !== "/") {
    throw new Error("Use Orion's gateway address without credentials, a path or query.");
  }
  return url.origin;
}

interface Probe {
  status(connection: GatewayConnection): Promise<GatewayStatus>;
  capabilities(connection: GatewayConnection): Promise<GatewayCapabilities>;
}

/** One polling loop, generation-guarded so stale requests cannot resurrect a connection. */
export class PairingController {
  private snapshot: ConnectionSnapshot = INITIAL;
  private listeners = new Set<() => void>();
  private target: GatewayConnection | null = null;
  private generation = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private failures = 0;
  private writes: Promise<unknown> = Promise.resolve();
  constructor(private store: PairingStore = nativePairingStore,
    private probe: Probe = { status: getStatus, capabilities: getCapabilities }) {
    this.snapshot = { ...INITIAL, persistent: store.persistent !== false };
  }
  current = () => this.snapshot;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private publish(update: Partial<ConnectionSnapshot>) {
    this.snapshot = { ...this.snapshot, ...update };
    this.listeners.forEach((listener) => listener());
  }
  private reset() {
    clearTimeout(this.timer); this.timer = undefined; this.failures = 0;
    return ++this.generation;
  }
  private offline(phase: ConnectionPhase, error: string | null = null) {
    this.publish({ phase, error, connection: null, status: null, capabilities: null });
  }
  private write(operation: () => Promise<void>) {
    const result = this.writes.then(operation);
    this.writes = result.catch(() => {});
    return result;
  }
  async start() {
    const generation = this.reset();
    this.offline("loading");
    try {
      await this.writes;
      const target = await this.store.load();
      if (generation !== this.generation) return;
      this.target = target;
      this.publish({ paired: !!target, address: target?.url ?? null });
      if (!target) { this.offline("unpaired"); return; }
      this.offline("connecting");
      await this.poll(generation);
    } catch (error) {
      if (generation === this.generation) this.offline("error", String(error instanceof Error ? error.message : error));
    }
  }
  async pair(address: string, token: string) {
    const generation = this.reset();
    this.offline("connecting");
    try {
      const target = { url: normalizeGatewayUrl(address), token: token.trim() };
      if (target.token.length < 32 || target.token.length > 4096) throw new Error("Enter Orion's complete pairing token.");
      const [status, capabilities] = await Promise.all([this.probe.status(target), this.probe.capabilities(target)]);
      if (generation !== this.generation) return false;
      await this.write(() => this.store.save(target));
      if (generation !== this.generation) return false;
      this.target = target;
      this.publish({ phase: "connected", paired: true, address: target.url,
        connection: target, status, capabilities, error: null });
      this.schedule(generation, 1000);
      return true;
    } catch (error) {
      if (generation === this.generation) this.offline("error", error instanceof GatewayError && [401, 403].includes(error.status)
        ? "Orion rejected this token. Check the token and pair again."
        : String(error instanceof Error ? error.message : error));
      return false;
    }
  }
  private schedule(generation: number, delay: number) {
    if (generation === this.generation) this.timer = setTimeout(() => { void this.poll(generation); }, delay);
  }
  private async poll(generation: number) {
    const target = this.target;
    if (!target || generation !== this.generation) return;
    try {
      // Capabilities only change when reconnecting; preserve catalog/viewport identity on heartbeat.
      const [status, capabilities] = await Promise.all([this.probe.status(target),
        this.snapshot.capabilities ?? this.probe.capabilities(target)]);
      if (generation !== this.generation) return;
      this.failures = 0;
      this.publish({ phase: "connected", connection: target, status, capabilities, error: null });
      this.schedule(generation, 1000);
    } catch (error) {
      if (generation !== this.generation) return;
      if (error instanceof GatewayError && [401, 403].includes(error.status)) {
        this.offline("auth_required", "Orion no longer accepts the saved token. Pair again.");
        return;
      }
      this.offline("reconnecting", "Orion is unavailable. Studio will reconnect automatically.");
      this.schedule(generation, Math.min(1000 * 2 ** this.failures++, 15000));
    }
  }
  reconnect() {
    if (!this.target) return this.start();
    const generation = this.reset();
    this.offline("connecting");
    return this.poll(generation);
  }
  disconnect() { this.reset(); this.offline("disconnected"); }
  async forget() {
    const generation = this.reset();
    this.offline("disconnected");
    try {
      await this.write(() => this.store.forget());
      if (generation !== this.generation) return;
      this.target = null;
      this.publish({ ...INITIAL, persistent: this.store.persistent !== false, phase: "unpaired" });
    } catch (error) {
      if (generation === this.generation) this.offline("error", String(error instanceof Error ? error.message : error));
    }
  }
  async refresh() {
    // Replace rather than overlap the existing heartbeat after a robot operation.
    if (this.snapshot.connection) await this.poll(this.reset());
  }
  dispose() { this.reset(); }
}
