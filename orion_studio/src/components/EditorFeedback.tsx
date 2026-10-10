import type { Failure } from "../lib/feedback";
import { FailureNotice } from "./FailureNotice";

export function EditorFeedback({ value }: { value: string | Failure | null }) {
  if (!value) return null;
  return typeof value === "string"
    ? <p role="status" className="editor-feedback">{value}</p>
    : <div className="editor-feedback"><FailureNotice value={value} /></div>;
}
