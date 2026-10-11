import { useCallback, useEffect, useRef, useState } from "react";
import { Camera } from "lucide-react";
import { getCameraDetections, getCameraSnapshot, setCameraEnabled, watchCamera,
  type CameraDetection, type CameraStreamState, type GatewayConnection } from "../lib/gateway";
import { CameraOverlay } from "../lib/cameraOverlay";
import "./CameraCard.css";

const labels: Record<CameraStreamState, string> = {
  connecting: "Connecting camera…", live: "Live", reconnecting: "Reconnecting camera…", off: "Camera is off", unavailable: "Camera unavailable",
};

export function drawCameraDetections(context: CanvasRenderingContext2D, detections: CameraDetection[], width: number, height: number) {
  context.clearRect(0, 0, width, height);
  context.font = "14px sans-serif";
  for (const detection of detections) {
    if (!Array.isArray(detection.box) || detection.box.length !== 4 || detection.box.some(v => !Number.isFinite(v) || v < 0 || v > 1) ||
        !Number.isSafeInteger(detection.track_id) || detection.track_id < 1 || !Number.isFinite(detection.score) ||
        detection.score < 0 || detection.score > 1 || typeof detection.chosen_target !== "boolean") continue;
    const [x, y, w, h] = detection.box;
    if (w <= 0 || h <= 0 || x + w > 1 + 1e-9 || y + h > 1 + 1e-9) continue;
    context.strokeStyle = detection.chosen_target ? "#ffe299" : "#78e6cf";
    context.fillStyle = context.strokeStyle; context.lineWidth = detection.chosen_target ? 3 : 2;
    context.strokeRect(x * width, y * height, w * width, h * height);
    context.fillText(`Face ${detection.track_id}${detection.chosen_target ? " · target" : ""}`, x * width + 4, Math.max(16, y * height - 5));
  }
}

export function CameraCard({ connection, overlay = true }: { connection: GatewayConnection | null; overlay?: boolean }) {
  const image = useRef<HTMLCanvasElement>(null);
  const boxes = useRef<HTMLCanvasElement>(null);
  const epoch = useRef(0);
  const overlayData = useRef(new CameraOverlay());
  const overlayEnabled = useRef(overlay); overlayEnabled.current = overlay;
  const live = useRef(false);
  const refreshDetections = useRef<(() => void) | null>(null);
  const paintOverlay = useCallback(() => {
    overlayData.current.render(overlayEnabled.current && live.current, detections => {
      const context = boxes.current?.getContext("2d");
      if (context) drawCameraDetections(context, detections, 640, 360);
    });
  }, []);
  const [state, setState] = useState<CameraStreamState>("connecting");
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [streamError, setStreamError] = useState("");
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    const generation = ++epoch.current;
    const controller = new AbortController();
    live.current = false; overlayData.current.clear(); paintOverlay();
    setEnabled(null); setState("connecting"); setError(""); setStreamError(""); setPending(false);
    image.current?.getContext("2d")?.clearRect(0, 0, 640, 360);
    if (connection) void watchCamera(connection, async frame => {
      const receivedAt = performance.now();
      const bitmap = await createImageBitmap(new Blob([new Uint8Array(frame.jpeg)], { type: "image/jpeg" }));
      try { if (generation === epoch.current && !controller.signal.aborted) {
        image.current?.getContext("2d")?.drawImage(bitmap, 0, 0, 640, 360);
        overlayData.current.frame(frame.captureBoottimeNs, receivedAt); live.current = true; paintOverlay();
        refreshDetections.current?.();
      } }
      finally { bitmap.close(); }
    }, (value, message) => {
      if (generation !== epoch.current || controller.signal.aborted) return;
      setState(value); setStreamError(message ?? "");
      live.current = value === "live";
      if (!live.current) overlayData.current.clear();
      paintOverlay();
      if (value === "live") setEnabled(true);
      if (value === "off") { setEnabled(false); image.current?.getContext("2d")?.clearRect(0, 0, 640, 360); }
    }, controller.signal);
    return () => { epoch.current++; controller.abort(); live.current = false; overlayData.current.clear(); paintOverlay(); };
  }, [connection, revision, paintOverlay]);

  useEffect(() => {
    const controller = new AbortController(); let timer: ReturnType<typeof setTimeout>; let inFlight = false;
    overlayData.current.clearDetections(); paintOverlay();
    const poll = async () => {
      if (!connection || !overlay || state !== "live" || inFlight || controller.signal.aborted) return;
      clearTimeout(timer); inFlight = true;
      const requestedAt = performance.now();
      try {
        const batch = await getCameraDetections(connection, AbortSignal.any([controller.signal, AbortSignal.timeout(300)]));
        if (!controller.signal.aborted) { overlayData.current.update(batch, requestedAt); paintOverlay(); }
      } catch { if (!controller.signal.aborted) { overlayData.current.clearDetections(); paintOverlay(); } }
      finally { inFlight = false; }
      if (!controller.signal.aborted) timer = setTimeout(() => void poll(), 100);
    };
    refreshDetections.current = () => void poll();
    void poll();
    return () => { controller.abort(); clearTimeout(timer); refreshDetections.current = null; overlayData.current.clearDetections(); paintOverlay(); };
  }, [connection, overlay, state, paintOverlay]);

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
