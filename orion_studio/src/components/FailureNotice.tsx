import type { Failure } from "../lib/feedback";

export function FailureNotice({ value }: { value: Failure | null }) {
  if (!value) return null;
  return <div className="failure-notice"><p role="alert" className="settings-error">{value.message}</p><details><summary>Technical details</summary><pre>{value.detail}</pre></details></div>;
}
