import { useEffect, useRef, useState } from "react";
import { RotateCcw, RotateCw, X } from "lucide-react";
import { getCameraLight, setCameraLight, type CameraLight, type GatewayConnection } from "../lib/gateway";
import "./CameraLightSetting.css";

const SIZE = 260, CENTRE = SIZE / 2, RADIUS = 104;
/** Where light `index` of `count` sits in the drawing: the first at the top, then clockwise. */
export function lightPoint(index: number, count: number) {
  const angle = (index / count) * 2 * Math.PI - Math.PI / 2;
  return { x: CENTRE + RADIUS * Math.cos(angle), y: CENTRE + RADIUS * Math.sin(angle) };
}
/** The light nearest to a point in the drawing. */
export function nearestLight(x: number, y: number, count: number) {
  const angle = Math.atan2(y - CENTRE, x - CENTRE) + Math.PI / 2;
  return ((Math.round((angle / (2 * Math.PI)) * count) % count) + count) % count;
}

type Status = { kind: "idle" | "saving" | "shown" | "saved" | "error"; text?: string };

/** The settings row and its window for placing the camera light on the ring. */
export function CameraLightSetting({ connection }: { connection: GatewayConnection | null }) {
  const [light, setLight] = useState<CameraLight | null>(null);
  const [open, setOpen] = useState(false);
  const [loadError, setLoadError] = useState("");
  useEffect(() => {
    if (!connection) { setLight(null); return; }
    const controller = new AbortController();
    setLoadError("");
    getCameraLight(connection, controller.signal).then(setLight).catch(error => {
      if (!controller.signal.aborted) setLoadError(error instanceof Error ? error.message : String(error));
    });
    return () => controller.abort();
  }, [connection]);
  return <div className="settings-row camera-light-row">
    <div><strong>Camera light</strong><p>{loadError ? "Update Orion to choose where the camera light shows." : "A small blue light shows when someone sees through Orion’s camera. Put it next to the camera so it’s easy to notice."}</p></div>
    <button className="quiet-button" type="button" disabled={!connection || !light} onClick={() => setOpen(true)}>Place the light</button>
    {open && light && connection && <CameraLightDialog connection={connection} light={light} onChange={setLight} onClose={() => setOpen(false)} />}
  </div>;
}

function CameraLightDialog({ connection, light, onChange, onClose }: { connection: GatewayConnection; light: CameraLight; onChange: (light: CameraLight) => void; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [position, setPosition] = useState(light.position);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const dragging = useRef(false);
  const sent = useRef(light.position);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => { dialog.current?.showModal(); return () => window.clearTimeout(timer.current); }, []);
  const show = (value: number) => {
    setStatus({ kind: "saving" });
    sent.current = value;
    setCameraLight(connection, value).then(result => {
      if (sent.current !== value) return;
      onChange(result);
      setStatus(result.shown ? { kind: "shown" } : { kind: "saved" });
    }).catch(error => {
      if (sent.current === value) setStatus({ kind: "error", text: error instanceof Error ? error.message : String(error) });
    });
  };
  const choose = (value: number, wait = 0) => {
    const next = ((value % light.count) + light.count) % light.count;
    setPosition(next);
    window.clearTimeout(timer.current);
    if (next === sent.current && status.kind !== "error") return;
    timer.current = window.setTimeout(() => show(next), wait);
  };
  const pointAt = (event: React.PointerEvent<SVGSVGElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    const x = ((event.clientX - box.left) / box.width) * SIZE, y = ((event.clientY - box.top) / box.height) * SIZE;
    choose(nearestLight(x, y, light.count), 150);
  };
  const message = status.kind === "saving" ? "Lighting it on Orion…"
    : status.kind === "shown" ? "Look at the lamp: this light is on for a few seconds."
    : status.kind === "saved" ? "Saved. Orion can’t show it right now because it is resting. It will use this light next time."
    : status.kind === "error" ? `Not saved. ${status.text}` : "Watch the lamp while you choose. Each time you move the light, Orion turns it on for a few seconds.";
  const point = lightPoint(position, light.count);
  return <dialog className="camera-light-dialog" ref={dialog} aria-labelledby="camera-light-title" onCancel={onClose} onClose={onClose}>
    <header><h2 id="camera-light-title">Place the camera light</h2><button className="icon-button" aria-label="Close" onClick={onClose}><X /></button></header>
    <p>Drag the blue light around the ring, or tap a spot, until the light on Orion sits next to the camera.</p>
    <svg className="camera-light-ring" viewBox={`0 0 ${SIZE} ${SIZE}`} role="slider" tabIndex={0} aria-label="Camera light position"
      aria-valuemin={1} aria-valuemax={light.count} aria-valuenow={position + 1} aria-valuetext={`Light ${position + 1} of ${light.count}`}
      onPointerDown={event => { dragging.current = true; event.currentTarget.setPointerCapture(event.pointerId); pointAt(event); }}
      onPointerMove={event => { if (dragging.current) pointAt(event); }}
      onPointerUp={() => { dragging.current = false; }} onPointerCancel={() => { dragging.current = false; }}
      onKeyDown={event => {
        if (event.key === "ArrowRight" || event.key === "ArrowUp") { event.preventDefault(); choose(position + 1, 150); }
        if (event.key === "ArrowLeft" || event.key === "ArrowDown") { event.preventDefault(); choose(position - 1, 150); }
      }}>
      <circle className="camera-light-track" cx={CENTRE} cy={CENTRE} r={RADIUS} />
      {Array.from({ length: light.count }, (_, index) => { const { x, y } = lightPoint(index, light.count);
        return <circle key={index} className="camera-light-led" cx={x} cy={y} r={7} />; })}
      <circle className="camera-light-glow" cx={point.x} cy={point.y} r={18} />
      <circle className="camera-light-chosen" cx={point.x} cy={point.y} r={9} />
      <text x={CENTRE} y={CENTRE - 4} textAnchor="middle" className="camera-light-label">Light {position + 1}</text>
      <text x={CENTRE} y={CENTRE + 16} textAnchor="middle" className="camera-light-sub">of {light.count}</text>
    </svg>
    <div className="camera-light-nudge">
      <button className="quiet-button" type="button" onClick={() => choose(position - 1)}><RotateCcw size={16} />Back</button>
      <button className="quiet-button" type="button" onClick={() => show(position)}>Show again</button>
      <button className="quiet-button" type="button" onClick={() => choose(position + 1)}>Forward<RotateCw size={16} /></button>
    </div>
    <p role="status" className={status.kind === "error" ? "settings-error" : "settings-help"}>{message}</p>
    <footer><span /><button className="primary-button" onClick={onClose}>Done</button></footer>
  </dialog>;
}
