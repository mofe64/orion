import { parseVoiceWorkerEvent, type VoiceWorkerEvent, type VoiceWorkerListener, type VoiceWorkerReadyEvent } from "./voiceWorkerProtocol";
import type { VoiceWorkerTransport } from "./studioVoicePipeline";

/** Authenticated observation only: the onboard coordinator owns audio and playback. */
export class GatewayVoiceClient implements VoiceWorkerTransport {
  private listeners = new Set<VoiceWorkerListener>();
  private abort = new AbortController();
  private generation: string | null = null;
  private sequence = 0;
  private closed = false;
  private pending: { resolve: (ready: VoiceWorkerReadyEvent) => void; reject: (error: Error) => void } | null = null;
  constructor(private readonly url: string, private readonly token: string,
    private readonly fetcher: typeof fetch = fetch, private readonly intervalMs = 250) {}
  connect(): Promise<VoiceWorkerReadyEvent> {
    if (this.closed) return Promise.reject(new Error("Voice observer is closed."));
    return new Promise((resolve, reject) => {
      this.pending = { resolve, reject };
      void this.poll();
    });
  }
  subscribe(listener: VoiceWorkerListener) { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  finishPlayback() { /* Playback acknowledgements are sent on the Pi. */ }
  close() {
    this.closed = true;
    this.abort.abort();
    this.pending?.reject(new Error("Voice observer closed."));
    this.pending = null;
    this.listeners.clear();
  }
  private async poll() {
    const deadline = Date.now() + 180_000;
    try {
      while (!this.closed) {
        const response = await this.fetcher(this.url, { headers: { Authorization: `Bearer ${this.token}` },
          redirect: "error", signal: AbortSignal.any([this.abort.signal, AbortSignal.timeout(5000)]) });
        if (!response.ok) throw new Error(`Orion voice connection failed (${response.status}).`);
        const raw = await response.text();
        if (raw.length > 1_048_576) throw new Error("Oversized Orion voice response.");
        const snapshot = JSON.parse(raw);
        if (!Array.isArray(snapshot.events) || snapshot.events.length > 33 ||
            (snapshot.generation !== null && typeof snapshot.generation !== "string")) throw new Error("Invalid Orion voice snapshot.");
        if (snapshot.generation !== this.generation) { this.sequence = 0; this.generation = snapshot.generation; }
        for (const value of snapshot.events) {
          if (!Number.isSafeInteger(value.eventId) || value.eventId < 1) throw new Error("Invalid Orion voice event sequence.");
          if (value.eventId <= this.sequence) continue;
          const event = parseVoiceWorkerEvent(JSON.stringify(value));
          if (event.type === "speech.chunk" || event.type === "speech.audio") throw new Error("Onboard observer cannot receive audio.");
          this.sequence = value.eventId;
          if (event.type === "ready") { this.pending?.resolve(event); this.pending = null; }
          if (event.type === "worker.error" && !event.recoverable) throw new Error(event.message);
          for (const listener of this.listeners) listener(event as VoiceWorkerEvent);
        }
        if (this.pending && Date.now() >= deadline) throw new Error("Orion voice models did not become ready.");
        await new Promise<void>(resolve => { const timer = setTimeout(done, this.intervalMs); const signal = this.abort.signal;
          function done() { clearTimeout(timer); signal.removeEventListener("abort", done); resolve(); }
          signal.addEventListener("abort", done, { once: true });
        });
      }
    } catch (error) {
      if (this.closed) return;
      const message = error instanceof Error ? error.message : String(error);
      this.pending?.reject(new Error(message)); this.pending = null;
      for (const listener of this.listeners) listener({ type: "worker.error", code: "observer_disconnected", message, recoverable: false });
    }
  }
}
