import { useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { Volume2, VolumeX } from "lucide-react";
import type { AudioStatus } from "../types";
import { setMasterVolume, type GatewayConnection } from "../lib/gateway";
import { createVolumeControl } from "../lib/volume";
import "./VolumeControl.css";

export function VolumeControl({connection, status}: {connection: GatewayConnection; status?: AudioStatus | null}) {
  const control = useMemo(() => createVolumeControl(level => setMasterVolume(connection, level), status), [connection]);
  const view = useSyncExternalStore(control.subscribe, control.snapshot, control.snapshot);
  const previous = useRef(40);
  useEffect(() => { control.receive(status); }, [control, status]);
  useEffect(() => () => control.cancel(), [control]);
  useEffect(() => { if (view.level) previous.current = view.level; }, [view.level]);
  const muted = view.level === 0;
  return <section className="orion-volume" aria-label="Volume controls">
    <div className="orion-volume-heading"><label htmlFor="orion-volume">Volume</label><output htmlFor="orion-volume">{view.level === null ? "–" : `${view.level}%`}</output></div>
    <div className="orion-volume-row">
      <button type="button" className="orion-volume-mute" aria-label={muted ? "Unmute" : "Mute"} aria-pressed={muted} disabled={view.level === null}
        onClick={() => control.choose(muted ? previous.current : 0)}>{muted ? <VolumeX size={18} /> : <Volume2 size={18} />}</button>
      <input id="orion-volume" type="range" min="0" max="100" step="1" value={view.level ?? 0} disabled={view.level === null}
        aria-valuetext={muted ? "Muted" : `${view.level}%`} aria-describedby="orion-volume-help"
        onPointerDown={control.begin} onKeyDown={control.begin} onChange={event => control.choose(Number(event.target.value))}
        onPointerUp={control.release} onPointerCancel={control.release} onKeyUp={control.release} onBlur={control.release} />
    </div>
    <p id="orion-volume-help">{view.level === null ? "Update Orion to change its volume." : "Alarms still sound when muted."}</p>
    {view.error && <p role="alert">{view.error}</p>}
  </section>;
}
