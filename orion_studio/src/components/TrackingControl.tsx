import { useId } from "react";
import { ScanFace } from "lucide-react";
import type { TrackingMode } from "../lib/gateway";
import type { TrackingState } from "../lib/trackingPreference";
import { FailureNotice } from "./FailureNotice";
import { failure } from "../lib/feedback";

interface Props extends TrackingState {
  connected: boolean;
  supported: boolean;
  onChange?: (mode: TrackingMode) => void;
  runtimeFailure?: string | null;
}
export function TrackingControl({ mode, cameraEnabled, pending, notice, error, connected, supported, onChange, runtimeFailure }: Props) {
  const id = useId();
  const disabled = !connected || !supported || cameraEnabled !== true || mode === null || pending;
  const label = !connected ? "Connect Orion to change tracking" : !supported ? "Camera unavailable on V1" :
    cameraEnabled === false ? "Camera is off · mode saved" : pending ? "Saving…" :
    runtimeFailure ? "Tracking paused · recovery needed" :
    mode === null ? error ? "Tracking setting unavailable" : "Checking tracking…" : mode === "faces" ? "Following faces" : mode === "hands" ? "Following your work" : "Tracking is off";
  return <div className="oh-input-control">
    <div className="oh-talk oh-tracking"><span className="oh-mic"><ScanFace size={19} /></span>
      <div className="oh-tracking-body"><strong id={`${id}-title`}>Tracking</strong><small id={`${id}-help`}>{label}</small>
        <fieldset className="oh-tracking-options" disabled={disabled} aria-labelledby={`${id}-title`} aria-describedby={`${id}-help`}>
          {(["off", "faces", "hands"] as const).map(value => <label key={value}><input type="radio" name={id} value={value}
            checked={mode === value} onChange={() => onChange?.(value)} /><span>{value === "off" ? "Off" : value === "hands" ? "Hands" : "Faces"}</span></label>)}
        </fieldset>
      </div>
    </div>
    {notice && !runtimeFailure && <p role="status" className="oh-feedback">{notice}</p>}<FailureNotice value={error} />
    {connected && supported && runtimeFailure && <FailureNotice value={failure(
      "Tracking paused after Orion could not complete a movement. Go to rest, then resume Orion.", runtimeFailure)} />}
  </div>;
}
