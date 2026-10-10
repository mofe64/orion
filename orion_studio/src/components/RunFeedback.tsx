import { assetDisplayName } from "../lib/displayName";
import { useEffect, useState } from "react";
import { cancelRun, type GatewayConnection } from "../lib/gateway";
import type { GatewayStatus, RunStatus } from "../types";
import { useScopedFeedback } from "../hooks/useScopedNotice";
import { failure, type Failure } from "../lib/feedback";
import { FailureNotice } from "./FailureNotice";
export type TrackedRun = { kind: "movement" | "scene" | "speech"; id: number; label: string };
export function acceptedRun(result: unknown, kind: TrackedRun["kind"], label: string): TrackedRun | null {
  const response = result as { result?: { run_id?: number; scene?: { run_id?: number } } };
  const id = response?.result?.run_id ?? response?.result?.scene?.run_id;
  return typeof id === "number" ? { kind, id, label } : null;
}
export function runStateLabel(state: string): string {
  return ({ queued: "Waiting to start", executing: "Playing", playing: "Playing", settling: "Finishing movement", completed: "Finished", cancelled: "Cancelled", failed: "Needs attention", stopped: "Stopped" } as Record<string, string>)[state] ?? "Updating activity";
}
export function RunFeedback({ status, connection, tracked, page }: { status: GatewayStatus | null; connection: GatewayConnection | null; tracked: TrackedRun | null; page: string }) {
  const [cancelling, setCancelling] = useState(false);
  const [feedback, showFeedback] = useScopedFeedback<string | Failure | null>(page, connection);
  const candidates: Array<[TrackedRun["kind"], RunStatus | null | undefined]> = [["scene", status?.scene.active], ["speech", status?.speech.active], ["movement", status?.runtime.motion]];
  const active = connection ? candidates.filter(([, run]) => run && !run.name?.startsWith("idle_")) : [];
  const history = tracked?.kind === "movement" ? status?.runtime.last_motion : tracked?.kind === "scene" ? status?.scene.last : status?.speech.last;
  const latest = tracked && history?.run_id === tracked.id ? history : null;
  const [observed, setObserved] = useState<{ kind: TrackedRun["kind"]; run: RunStatus } | null>(null);
  useEffect(() => {
    if (tracked && latest) setObserved({ kind: tracked.kind, run: latest });
  }, [tracked, latest]);
  // Runtime history is bounded: background motion can replace last_motion.
  const result = latest ?? (observed && tracked && observed.kind === tracked.kind && observed.run.run_id === tracked.id ? observed.run : null);
  const knownResult = result && ["completed", "cancelled", "failed", "stopped"].includes(result.state);
  if (!active.length && !tracked) return null;
  return <section className="run-feedback" aria-label="Orion activity" aria-live="polite">
    {active.length ? active.map(([kind, run]) => run && <div key={`${kind}:${run.run_id}`}><span><strong>{(run.name ? assetDisplayName(run.name) : undefined) ?? (kind === "movement" ? "Movement" : kind === "scene" ? "Scene" : "Voice reply")}</strong> · {runStateLabel(run.state)}</span><button disabled={!connection || cancelling} onClick={async () => {
      if (!connection) return;
      setCancelling(true); showFeedback(null);
      try { await cancelRun(connection, kind, run.run_id); showFeedback("Cancellation requested."); }
      catch (error) { showFeedback(failure("Studio couldn't cancel this activity. Please try again.", error)); }
      finally { setCancelling(false); }
    }}>{cancelling ? "Cancelling…" : "Cancel"}</button></div>) : <p>{tracked?.label} · {!connection && !knownResult ? "Connection lost; completion unknown" : result ? runStateLabel(result.state) : "Waiting for activity status"}</p>}
    {typeof feedback === "string" ? <p>{feedback}</p> : <FailureNotice value={feedback} />}
    {connection && result?.error && <FailureNotice value={failure("Orion couldn't finish this activity.", result.error)} />}
  </section>;
}
