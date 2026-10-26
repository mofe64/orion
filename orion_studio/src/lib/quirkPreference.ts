import { failure, type Failure } from "./feedback";
import { getPersonality, setPersonality, type GatewayConnection, type Quirks } from "./gateway";
import { validQuirks } from "../components/QuirkSetting";

export interface QuirkState { values: Quirks | null; pending: boolean; notice: string; error: Failure | null }
export const emptyQuirks: QuirkState = { values: null, pending: false, notice: "", error: null };
/** Orion's quirks, read once when Settings opens and saved on each choice. */
export class QuirkPreference {
  private state: QuirkState = emptyQuirks;
  private listeners = new Set<() => void>();
  private controller: AbortController | null = null;
  constructor(private connection: GatewayConnection, private transport = { read: getPersonality, save: setPersonality }) {}
  snapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(patch: Partial<QuirkState>) { this.state = { ...this.state, ...patch }; this.listeners.forEach(listener => listener()); }
  private validate(value: { quirks?: unknown }) {
    if (!validQuirks(value?.quirks)) throw new Error("The gateway returned an invalid personality.");
    return value.quirks;
  }
  async start() {
    this.stop();
    const controller = new AbortController(); this.controller = controller;
    try {
      const values = this.validate(await this.transport.read(this.connection, AbortSignal.any([controller.signal, AbortSignal.timeout(5000)])));
      if (!controller.signal.aborted) this.update({ values, error: null });
    } catch (error) {
      if (!controller.signal.aborted) this.update({ error: failure("Studio couldn't read Orion's quirks. Please reconnect.", error) });
    }
  }
  stop() {
    this.controller?.abort(); this.controller = null;
    if (this.state.pending) this.update({ values: null, pending: false, notice: "" });
  }
  choose = async <K extends keyof Quirks>(key: K, value: Quirks[K]) => {
    const controller = this.controller, previous = this.state.values;
    if (!controller || this.state.pending || !previous || previous[key] === value) return;
    this.update({ values: { ...previous, [key]: value }, pending: true, error: null, notice: "" });
    try {
      const values = this.validate(await this.transport.save(this.connection, { [key]: value }, AbortSignal.any([controller.signal, AbortSignal.timeout(5000)])));
      if (!controller.signal.aborted) this.update({ values, notice: "Saved on Orion." });
    } catch (error) {
      if (!controller.signal.aborted) this.update({ values: previous, error: failure("Studio couldn't save this quirk. Please try again.", error) });
    } finally { if (!controller.signal.aborted) this.update({ pending: false }); }
  };
}
