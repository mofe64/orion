import { failure, type Failure } from "./feedback";
import { getTrackingMode, setTrackingMode, type GatewayConnection, type TrackingMode, type TrackingSetting } from "./gateway";

export interface TrackingState {
  mode: TrackingMode | null;
  cameraEnabled: boolean | null;
  pending: boolean;
  notice: string;
  error: Failure | null;
}
type Transport = { read: typeof getTrackingMode; save: typeof setTrackingMode };

// One instance belongs to one connection. Revision checks prevent an older
// preference read from replacing an optimistic selection or its saved result.
export class TrackingPreference {
  private state: TrackingState = { mode: null, cameraEnabled: null, pending: false, notice: "", error: null };
  private listeners = new Set<() => void>();
  private controller: AbortController | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private revision = 0;
  private cameraRevision = 0;
  private saveFailure: Failure | null = null;
  constructor(private connection: GatewayConnection, private transport: Transport = { read: getTrackingMode, save: setTrackingMode }) {}
  snapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(value: Partial<TrackingState>) {
    this.state = { ...this.state, ...value };
    this.listeners.forEach(listener => listener());
  }
  private validate(value: TrackingSetting) {
    if (!["off", "faces", "hands"].includes(value.tracking_mode) || typeof value.camera_enabled !== "boolean")
      throw new Error("The gateway returned an invalid tracking setting.");
    return value;
  }
  start() {
    this.stop();
    const controller = new AbortController(); this.controller = controller;
    const poll = async () => {
      const revision = this.revision;
      try {
        const saved = this.validate(await this.transport.read(this.connection,
          AbortSignal.any([controller.signal, AbortSignal.timeout(5000)])));
        if (!controller.signal.aborted && revision === this.revision && !this.state.pending)
          this.update({ mode: saved.tracking_mode, cameraEnabled: saved.camera_enabled, error: this.saveFailure,
            notice: saved.tracking_mode === this.state.mode ? this.state.notice : "" });
      } catch (error) {
        if (!controller.signal.aborted && revision === this.revision && !this.state.pending)
          this.update({ error: this.saveFailure ?? failure("Studio couldn't read Orion's tracking setting. Please try reconnecting.", error) });
      }
      if (!controller.signal.aborted) this.timer = setTimeout(() => void poll(), 2000);
    };
    void poll();
  }
  stop() {
    this.controller?.abort(); this.controller = null;
    clearTimeout(this.timer);
    this.revision++;
  }
  cameraChanged = (enabled: boolean) => {
    if (enabled === this.state.cameraEnabled) return;
    this.revision++;
    this.cameraRevision++;
    this.update({ cameraEnabled: enabled });
  };
  choose = async (mode: TrackingMode) => {
    const controller = this.controller;
    if (!controller || this.state.pending || this.state.mode === null || !this.state.cameraEnabled || mode === this.state.mode) return;
    const previous = this.state.mode;
    const cameraRevision = this.cameraRevision;
    // Reads are invalidated, while the independent camera state may still change.
    this.revision++;
    this.saveFailure = null;
    this.update({ mode, pending: true, notice: "", error: null });
    try {
      const saved = this.validate(await this.transport.save(this.connection, mode,
        AbortSignal.any([controller.signal, AbortSignal.timeout(5000)])));
      if (!controller.signal.aborted)
        this.update({ mode: saved.tracking_mode,
          ...(cameraRevision === this.cameraRevision ? { cameraEnabled: saved.camera_enabled } : {}),
          notice: saved.tracking_mode === "off" ? "Tracking is off." : saved.tracking_mode === "hands" ? "Following your work." : "Face tracking is on." });
    } catch (error) {
      if (!controller.signal.aborted) {
        this.saveFailure = failure("Studio couldn't save Orion's tracking mode. Please try again.", error);
        this.update({ mode: previous, error: this.saveFailure });
      }
    } finally {
      if (!controller.signal.aborted) { this.revision++; this.update({ pending: false }); }
    }
  };
}
