import { useId } from "react";
import { ScanFace } from "lucide-react";
import type { TrackingMode } from "../lib/gateway";
import type { TrackingState } from "../lib/trackingPreference";
import { FailureNotice } from "./FailureNotice";

interface Props extends TrackingState {
  connected: boolean;
  supported: boolean;
  onChange?: (mode: TrackingMode) => void;
}
export function TrackingControl({ mode, cameraEnabled, pending, notice, error, connected, supported, onChange }: Props) {
  const id = useId();
  const disabled = !connected || !supported || cameraEnabled !== true || mode === null || pending;
  const label = !connected ? "Connect Orion to change tracking" : !supported ? "Camera unavailable on V1" :
    cameraEnabled === false ? "Camera is off · mode saved" : pending ? "Saving…" :
    mode === null ? error ? "Tracking setting unavailable" : "Checking tracking…" : mode === "idle" ? "Autonomous behaviours · conversation gaze" : mode === "faces" ? "Following faces" : mode === "hands" ? "Following hands" : "Tracking is off";
  return <div className="oh-input-control">
    <div className="oh-talk oh-tracking"><span className="oh-mic"><ScanFace size={19} /></span>
      <div className="oh-tracking-body"><strong id={`${id}-title`}>Tracking</strong><small id={`${id}-help`}>{label}</small>
        <fieldset className="oh-tracking-options" disabled={disabled} aria-labelledby={`${id}-title`} aria-describedby={`${id}-help`}>
          {(["off", "idle", "faces", "hands"] as const).map(value => <label key={value}><input type="radio" name={id} value={value}
            checked={mode === value} onChange={() => onChange?.(value)} /><span>{value === "off" ? "Off" : value === "idle" ? "Idle" : value === "hands" ? "Hands" : "Faces"}</span></label>)}
        </fieldset>
      </div>
    </div>
    {notice && <p role="status" className="oh-feedback">{notice}</p>}<FailureNotice value={error} />
  </div>;
}
