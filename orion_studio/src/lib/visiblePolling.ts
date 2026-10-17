export function visiblePolling(task: (signal: AbortSignal) => Promise<void>, visibility: Pick<Document, "hidden" | "addEventListener" | "removeEventListener"> = document, interval = 2000): () => void {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let request: AbortController | undefined;
  const poll = async () => {
    if (stopped || visibility.hidden || request) return;
    const current = new AbortController();
    request = current;
    try { await task(current.signal); }
    catch { /* The page owns request errors. */ }
    finally {
      if (request === current) {
        request = undefined;
        if (!stopped && !visibility.hidden) timer = setTimeout(() => void poll(), interval);
      }
    }
  };
  const change = () => {
    clearTimeout(timer);
    request?.abort(); request = undefined;
    if (!visibility.hidden) void poll();
  };
  visibility.addEventListener("visibilitychange", change);
  void poll();
  return () => { stopped = true; clearTimeout(timer); request?.abort(); visibility.removeEventListener("visibilitychange", change); };
}
