import { failure, type Failure } from "./feedback";
import { behaviourKeys, getBehaviours, setBehaviours, type BehaviourKey, type BehaviourResponse, type BehaviourSettings, type GatewayConnection } from "./gateway";

export interface BehaviourState { values: BehaviourSettings | null; pending: boolean; notice: string; error: Failure | null }
export const emptyBehaviours: BehaviourState = { values: null, pending: false, notice: "", error: null };
export class BehaviourPreference {
  private state: BehaviourState = emptyBehaviours;
  private listeners = new Set<() => void>();
  private controller: AbortController | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private revision = 0;
  private saveFailure: Failure | null = null;
  constructor(private connection: GatewayConnection, private transport = { read: getBehaviours, save: setBehaviours }) {}
  snapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(patch: Partial<BehaviourState>) { this.state = { ...this.state, ...patch }; this.listeners.forEach(listener => listener()); }
  private validate(value: BehaviourResponse) {
    if (!value.behaviours || behaviourKeys.some(key => typeof value.behaviours[key] !== "boolean"))
      throw new Error("The gateway returned invalid character behaviours.");
    return value.behaviours;
  }
  start() {
    this.stop();
    const controller = new AbortController(); this.controller = controller;
    const poll = async () => {
      const revision = this.revision;
      try {
        const values = this.validate(await this.transport.read(this.connection, AbortSignal.any([controller.signal, AbortSignal.timeout(5000)])));
        if (!controller.signal.aborted && revision === this.revision && !this.state.pending)
          this.update({ values, error: this.saveFailure });
      } catch (error) {
        if (!controller.signal.aborted && revision === this.revision && !this.state.pending)
          this.update({ error: this.saveFailure ?? failure("Studio couldn't read Orion's character behaviours. Please reconnect.", error) });
      }
      if (!controller.signal.aborted) this.timer = setTimeout(() => void poll(), 2000);
    };
    void poll();
  }
  stop() { this.controller?.abort(); this.controller = null; clearTimeout(this.timer); this.revision++; }
  choose = async (key: BehaviourKey, value: boolean) => {
    const controller = this.controller, previous = this.state.values;
    if (!controller || this.state.pending || !previous || previous[key] === value) return;
    this.revision++; this.saveFailure = null;
    this.update({ values: { ...previous, [key]: value }, pending: true, error: null, notice: "" });
    try {
      const values = this.validate(await this.transport.save(this.connection, { [key]: value }, AbortSignal.any([controller.signal, AbortSignal.timeout(5000)])));
      if (!controller.signal.aborted) this.update({ values, notice: "Saved on Orion." });
    } catch (error) {
      if (!controller.signal.aborted) {
        this.saveFailure = failure("Studio couldn't save this behaviour. Please try again.", error);
        this.update({ values: previous, error: this.saveFailure });
      }
    } finally { if (!controller.signal.aborted) { this.revision++; this.update({ pending: false }); } }
  };
}
