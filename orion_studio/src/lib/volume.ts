import type { AudioStatus } from "../types";

export function createVolumeControl(send: (level: number) => Promise<AudioStatus>, initial?: AudioStatus | null) {
  let actual = initial ?? null;
  let view = {level: actual?.volume ?? null, error: actual?.save_error ?? null};
  let dragging = false, sending = false, queued: number | null = null, generation = 0;
  const retiredEpochs = new Set<string>();
  const listeners = new Set<() => void>();
  const publish = (level = view.level, error: string | null = actual?.save_error ?? null) => {
    view = {level, error}; listeners.forEach(listener => listener());
  };
  const receive = (status?: AudioStatus | null) => {
    if (!status || retiredEpochs.has(status.epoch)) return;
    if (actual?.epoch === status.epoch && status.revision < actual.revision) return;
    if (actual && actual.epoch !== status.epoch) retiredEpochs.add(actual.epoch);
    actual = status;
    if (!dragging && !sending && queued === null) publish(actual.volume);
  };
  const flush = async () => {
    if (sending || queued === null) return;
    const level = queued, id = generation;
    queued = null; sending = true;
    try {
      const result = await send(level);
      if (generation === id) receive(result);
    } catch {
      if (generation === id) publish(view.level, "Studio couldn't change the volume. Please try again.");
    } finally {
      if (generation === id) {
        sending = false;
        if (queued !== null) void flush();
        else if (!dragging) publish(actual?.volume ?? null, view.error ?? actual?.save_error ?? null);
      }
    }
  };
  return {
    snapshot: () => view,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    receive,
    begin: () => { dragging = true; },
    choose: (level: number) => {
      if (!Number.isInteger(level) || level < 0 || level > 100) return;
      queued = level; publish(level, null); void flush();
    },
    release: () => {
      dragging = false;
      if (!sending && queued === null) publish(actual?.volume ?? null, view.error ?? actual?.save_error ?? null);
    },
    cancel: () => { generation++; dragging = false; sending = false; queued = null; },
  };
}
