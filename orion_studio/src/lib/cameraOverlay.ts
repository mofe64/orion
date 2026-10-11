import type { CameraDetection, CameraDetectionBatch } from "./gateway";

const MAX_AGE_MS = 300;

/** One displayed frame and one detection batch, both memory-only. Pi capture
 * age plus the whole request duration conservatively includes network delay. */
export class CameraOverlay {
  private stamp: string | null = null;
  private frameReceivedAt = 0;
  private batch: CameraDetectionBatch | null = null;
  private requestedAt = 0;
  private expiry: ReturnType<typeof setTimeout> | null = null;

  frame(stamp: string | null, receivedAt: number) {
    if (this.stamp !== stamp) this.frameReceivedAt = receivedAt;
    this.stamp = stamp;
  }

  update(batch: CameraDetectionBatch, requestedAt: number) {
    this.batch = batch;
    this.requestedAt = requestedAt;
  }

  clearDetections() { this.batch = null; }
  clear() {
    this.stamp = null; this.clearDetections();
    if (this.expiry !== null) clearTimeout(this.expiry);
    this.expiry = null;
  }

  render(enabled: boolean, draw: (detections: CameraDetection[]) => void, clock = () => performance.now()) {
    if (this.expiry !== null) clearTimeout(this.expiry);
    this.expiry = null;
    const view = enabled ? this.view(clock()) : { detections: [], expiresIn: null };
    draw(view.detections);
    if (view.expiresIn !== null) this.expiry = setTimeout(() => this.render(enabled, draw, clock), view.expiresIn);
  }

  view(now: number): { detections: CameraDetection[]; expiresIn: number | null } {
    const empty = { detections: [], expiresIn: null };
    const batch = this.batch;
    if (!this.stamp || !/^\d+$/.test(this.stamp) || !batch || batch.capture_boottime_ns !== this.stamp ||
        batch.age_ms === null || !Number.isFinite(batch.age_ms) || batch.age_ms < 0 || !Array.isArray(batch.detections)) return empty;
    const age = Math.max(now - this.frameReceivedAt, batch.age_ms + Math.max(0, now - this.requestedAt));
    if (age >= MAX_AGE_MS) return empty;
    return { detections: batch.detections.slice(0, 16).filter(value => value?.capture_boottime_ns === this.stamp),
      expiresIn: MAX_AGE_MS - age };
  }
}
