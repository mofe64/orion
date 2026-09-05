import { useEffect, useState } from "react";
import { cancelRun, type GatewayConnection } from "../lib/gateway";
import type { GatewayStatus, RunStatus } from "../types";
export type TrackedRun = { kind: "movement" | "scene" | "speech"; id: number; label: string };
export function acceptedRun(result: unknown, kind: TrackedRun["kind"], label: string): TrackedRun | null {
  const response = result as { result?: { run_id?: number; scene?: { run_id?: number } } };
  const id = response?.result?.run_id ?? response?.result?.scene?.run_id;
  return typeof id === "number" ? { kind, id, label } : null;
}
export function RunFeedback({ status, connection, tracked, onNotice }: { status: GatewayStatus | null; connection: GatewayConnection | null; tracked: TrackedRun | null; onNotice: (text: string) => void }) {
  const [cancelling, setCancelling] = useState(false);
  const candidates: Array<[TrackedRun["kind"], RunStatus | null | undefined]> = [["scene", status?.scene.active], ["speech", status?.speech.active], ["movement", status?.runtime.motion]];
  const active = candidates.filter(([, run]) => run && !run.name?.startsWith("idle_"));
  const history = tracked?.kind === "movement" ? status?.runtime.last_motion : tracked?.kind === "scene" ? status?.scene.last : status?.speech.last;
  const latest = tracked && history?.run_id === tracked.id ? history : null;
  const [observed, setObserved] = useState<{ kind: TrackedRun["kind"]; run: RunStatus } | null>(null);
  useEffect(() => {
    if (tracked && latest) setObserved({ kind: tracked.kind, run: latest });
  }, [tracked, latest]);
  // Runtime history is bounded: background motion can replace last_motion.
  const result = latest ?? (observed && tracked && observed.kind === tracked.kind && observed.run.run_id === tracked.id ? observed.run : null);
  if (!active.length && !tracked) return null;
  return <section className="run-feedback" aria-label="Robot activity" aria-live="polite">
    {active.length ? active.map(([kind, run]) => run && <div key={`${kind}:${run.run_id}`}><span><strong>{run.name?.replaceAll("_", " ") ?? kind}</strong> · {run.state} · run {run.run_id}</span><button disabled={!connection || cancelling} onClick={async () => {
      if (!connection) return;
      setCancelling(true);
      try { await cancelRun(connection, kind, run.run_id); onNotice(`Cancellation requested for ${kind} ${run.run_id}.`); }
      catch (error) { onNotice(error instanceof Error ? error.message : String(error)); }
      finally { setCancelling(false); }
    }}>{cancelling ? "Cancelling…" : `Cancel ${kind}`}</button></div>) : <p>{tracked?.label} · {result?.state ?? (connection ? "Waiting for run status" : "Connection lost; completion unknown")}{result?.error ? ` · ${result.error}` : ""}</p>}
  </section>;
}
