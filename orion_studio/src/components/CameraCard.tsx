import { useEffect, useRef, useState } from "react";
import { Camera } from "lucide-react";
import { getCameraObservations, getCameraSnapshot, setCameraEnabled, watchCamera,
  type CameraObservation, type CameraStreamState, type GatewayConnection } from "../lib/gateway";
import "./CameraCard.css";

const labels: Record<CameraStreamState, string> = {
  connecting: "Connecting camera…", live: "Live", reconnecting: "Reconnecting camera…", off: "Camera is off", unavailable: "Camera unavailable",
};

export function drawCameraObservations(context: CanvasRenderingContext2D, observations: CameraObservation[], width: number, height: number) {
  context.clearRect(0, 0, width, height);
  context.strokeStyle = "#78e6cf"; context.fillStyle = "#78e6cf"; context.lineWidth = 2; context.font = "14px sans-serif";
  for (const observation of observations) {
    if (!Array.isArray(observation.box) || observation.box.length !== 4 || observation.box.some(v => !Number.isFinite(v) || v < 0 || v > 1)) continue;
    const [x, y, w, h] = observation.box;
    context.strokeRect(x * width, y * height, w * width, h * height);
    context.fillText(`${observation.class} · ${observation.id}`, x * width + 4, Math.max(16, y * height - 5));
  }
}

export function CameraCard({ connection, overlay = false }: { connection: GatewayConnection | null; overlay?: boolean }) {
  const image = useRef<HTMLCanvasElement>(null);
  const boxes = useRef<HTMLCanvasElement>(null);
  const epoch = useRef(0);
  const [state, setState] = useState<CameraStreamState>("connecting");
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [streamError, setStreamError] = useState("");
  const [revision, setRevision] = useState(0);
  const [observations, setObservations] = useState<CameraObservation[]>([]);

  useEffect(() => {
    const generation = ++epoch.current;
    const controller = new AbortController();
    setEnabled(null); setState("connecting"); setError(""); setStreamError(""); setPending(false);
    image.current?.getContext("2d")?.clearRect(0, 0, 640, 360);
    if (connection) void watchCamera(connection, async frame => {
      const bitmap = await createImageBitmap(new Blob([new Uint8Array(frame.jpeg)], { type: "image/jpeg" }));
      try { if (generation === epoch.current && !controller.signal.aborted) image.current?.getContext("2d")?.drawImage(bitmap, 0, 0, 640, 360); }
      finally { bitmap.close(); }
    }, (value, message) => {
      if (generation !== epoch.current || controller.signal.aborted) return;
      setState(value); setStreamError(message ?? "");
      if (value === "live") setEnabled(true);
      if (value === "off") { setEnabled(false); image.current?.getContext("2d")?.clearRect(0, 0, 640, 360); }
    }, controller.signal);
    return () => { epoch.current++; controller.abort(); };
  }, [connection, revision]);

  useEffect(() => {
    const controller = new AbortController(); let timer: ReturnType<typeof setTimeout>;
    setObservations([]);
    const poll = async () => {
      if (!connection || !overlay || state !== "live") return;
      try {
        const values = await getCameraObservations(connection, AbortSignal.any([controller.signal, AbortSignal.timeout(3000)]));
        if (!controller.signal.aborted) setObservations(values);
      } catch { if (!controller.signal.aborted) setObservations([]); }
      if (!controller.signal.aborted) timer = setTimeout(() => void poll(), 500);
    };
    void poll();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [connection, overlay, state]);
  useEffect(() => {
    const context = boxes.current?.getContext("2d");
    if (context) drawCameraObservations(context, overlay && state === "live" ? observations : [], 640, 360);
  }, [observations, overlay, state]);

  const toggle = async (desired?: boolean) => {
    if (!connection || desired === undefined && enabled === null || pending) return;
    const generation = epoch.current; setPending(true); setError("");
    try {
      const saved = await setCameraEnabled(connection, desired ?? !enabled);
      if (generation !== epoch.current) return;
      setEnabled(saved.camera_enabled); setRevision(value => value + 1);
    } catch (value) { if (generation === epoch.current) setError(value instanceof Error ? value.message : "Couldn’t change camera privacy."); }
    finally { if (generation === epoch.current) setPending(false); }
  };
  const snapshot = async () => {
    if (!connection || pending || state !== "live") return;
    const generation = epoch.current; setPending(true); setError("");
    try {
      const blob = await getCameraSnapshot(connection);
      if (generation !== epoch.current) return;
      const url = URL.createObjectURL(blob); const link = document.createElement("a");
      link.href = url; link.download = `orion-camera-${new Date().toISOString().replace(/[:.]/g, "-")}.jpg`; link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (value) { if (generation === epoch.current) setError(value instanceof Error ? value.message : "Couldn’t save snapshot."); }
    finally { if (generation === epoch.current) setPending(false); }
  };
  return <section className="camera-card" aria-label="Orion camera" aria-busy={pending}>
    <header><h2><Camera size={19} />Camera</h2><button className="studio-switch" role="switch" aria-label="Camera capture" aria-checked={enabled === true} disabled={!connection || enabled === null || pending} onClick={() => void toggle()}><span /></button></header>
    <div className="camera-picture"><canvas ref={image} width={640} height={360} aria-label="Live Orion camera image" /><canvas ref={boxes} className="camera-overlay" width={640} height={360} aria-hidden="true" />
      {state !== "live" && <div className="camera-placeholder">{connection ? labels[state] : "Connect Orion to view its camera"}</div>}</div>
    <div className="camera-actions"><p role="status">{connection ? labels[state] : "Camera disconnected"}</p><button className="quiet-button" disabled={!connection || pending || state !== "live"} onClick={() => void snapshot()}>Save snapshot</button>
      {connection && enabled === null && <button className="quiet-button" disabled={pending} onClick={() => void toggle(false)}>Turn camera off</button>}</div>
    <p className="camera-help">Turning the camera off stops capture for everyone. Snapshots save to this device only when you choose Save snapshot.</p>
    {(error || streamError) && <p role="alert" className="camera-error">{error || streamError}</p>}
  </section>;
}
