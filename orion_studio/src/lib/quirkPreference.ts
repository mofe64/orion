import { failure, type Failure } from "./feedback";
import { getPersonality, setPersonality, type GatewayConnection, type LivelyRules, type PersonalityChange, type Quirks } from "./gateway";
import { validQuirks } from "../components/QuirkSetting";

export interface QuirkState { values: Quirks | null; rules: LivelyRules | null; pending: boolean; notice: string; error: Failure | null }
export const emptyQuirks: QuirkState = { values: null, rules: null, pending: false, notice: "", error: null };
const validRules = (value: unknown): value is LivelyRules => !!value && typeof value === "object"
  && typeof (value as LivelyRules).per_hour === "number" && typeof (value as LivelyRules).looks === "boolean";
/** Orion's quirks and Lively rules, read once when Settings opens and saved on each choice. */
export class QuirkPreference {
  private state: QuirkState = emptyQuirks;
  private listeners = new Set<() => void>();
  private controller: AbortController | null = null;
  constructor(private connection: GatewayConnection, private transport = { read: getPersonality, save: setPersonality }) {}
  snapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(patch: Partial<QuirkState>) { this.state = { ...this.state, ...patch }; this.listeners.forEach(listener => listener()); }
  private validate(value: { quirks?: unknown; rules?: unknown }) {
    if (!validQuirks(value?.quirks)) throw new Error("The gateway returned an invalid personality.");
    return { values: value.quirks, rules: validRules(value.rules) ? value.rules : null };
  }
  async start() {
    this.stop();
    const controller = new AbortController(); this.controller = controller;
    try {
      const values = this.validate(await this.transport.read(this.connection, AbortSignal.any([controller.signal, AbortSignal.timeout(5000)])));
      if (!controller.signal.aborted) this.update({ ...values, error: null });
    } catch (error) {
      if (!controller.signal.aborted) this.update({ error: failure("Studio couldn't read Orion's quirks. Please reconnect.", error) });
    }
  }
  stop() {
    this.controller?.abort(); this.controller = null;
    if (this.state.pending) this.update({ values: null, rules: null, pending: false, notice: "" });
  }
  private async save(change: PersonalityChange, optimistic: Partial<QuirkState>, message: string) {
    const controller = this.controller, previous = { values: this.state.values, rules: this.state.rules };
    if (!controller || this.state.pending || !previous.values) return;
    this.update({ ...optimistic, pending: true, error: null, notice: "" });
    try {
      const values = this.validate(await this.transport.save(this.connection, change, AbortSignal.any([controller.signal, AbortSignal.timeout(5000)])));
      if (!controller.signal.aborted) this.update({ ...values, notice: "Saved on Orion." });
    } catch (error) {
      if (!controller.signal.aborted) this.update({ ...previous, error: failure(message, error) });
    } finally { if (!controller.signal.aborted) this.update({ pending: false }); }
  }
  choose = async <K extends keyof Quirks>(key: K, value: Quirks[K]) => {
    const values = this.state.values;
    if (!values || values[key] === value) return;
    await this.save({ quirks: { [key]: value } }, { values: { ...values, [key]: value } }, "Studio couldn't save this quirk. Please try again.");
  };
  /** Changes several rules at once, as when going back to the usual rules. */
  chooseRules = async (rules: Partial<LivelyRules>) => {
    const current = this.state.rules;
    if (!current) return;
    await this.save({ rules }, { rules: { ...current, ...rules } }, "Studio couldn't save this change. Please try again.");
  };
}
