import { useEffect, useState } from "react";
import { getSystemMetrics, type GatewayConnection } from "../lib/gateway";
import { bytes, metricValue, type SystemMetrics } from "../lib/systemMetrics";
import { visiblePolling } from "../lib/visiblePolling";
import "./Settings.css";
import "./System.css";

function Sparkline({ points, label }: { points: [number, number | null][]; label: string }) {
  const end = points.at(-1)?.[0] ?? 0;
  const paths: string[] = [];
  let segment = "";
  for (const [time, value] of points) {
    if (value == null) { if (segment) paths.push(segment); segment = ""; continue; }
    segment += `${segment ? "L" : "M"}${Math.max(0, (time - end + 900) / 900 * 300).toFixed(2)},${(60 - Math.min(100, value) / 100 * 56).toFixed(2)} `;
  }
  if (segment) paths.push(segment);
  return <svg viewBox="0 0 300 64" role="img" aria-label={`${label}, last 15 minutes`} preserveAspectRatio="none"><title>{label}, last 15 minutes</title>{paths.map((path, i) => <path key={i} d={path} fill="none" stroke="currentColor" strokeWidth="2" vectorEffect="non-scaling-stroke" />)}</svg>;
}
const PARTS: Record<string, string> = {
  oriond: "Movement and lights",
  "orion-listener": "Listening",
  "orion-voice-stack": "Speech and replies",
  "orion-studio-gateway": "Studio connection",
  "orion-vision": "Camera",
  "orion-home-assistant": "Home Assistant",
};
const STATES: Record<string, string> = { active: "Running", activating: "Starting", reloading: "Starting", deactivating: "Stopping", inactive: "Stopped", failed: "Stopped with a problem" };
function partState(state: string | null, restarts: number | null) {
  const label = state ? STATES[state] ?? "Unknown" : "Unknown";
  return restarts ? `${label} · restarted ${restarts} ${restarts === 1 ? "time" : "times"}` : label;
}
export function SystemReadings({ metrics }: { metrics: SystemMetrics }) {
  const value = metrics.latest;
  if (!value) return <p role="status">Loading…</p>;
  const now = value.cpu.throttling?.now ?? {};
  const warnings = new Set([
    ...(now.under_voltage ? ["Orion is not getting enough power. Check the power supply."] : []),
    ...(now.throttled || now.frequency_capped || now.soft_temperature_limit ? ["Orion is slowing down to protect itself."] : []),
    ...(!Object.values(now).some(Boolean) && Object.values(value.cpu.throttling?.since_boot ?? {}).some(Boolean) ? ["Orion slowed down earlier because of low power or heat."] : []),
    ...(value.cpu.temperature_c != null && value.cpu.temperature_c > 75 ? ["Orion is running hot. Give it space to cool down."] : []),
    ...(Object.values(value.disk).some(disk => disk.percent != null && disk.percent > 90) ? ["Storage is almost full."] : []),
  ]);
  const tiles = [
    ["Processor", metricValue(value.cpu.percent), "cpu", "How busy Orion is"],
    ["Temperature", metricValue(value.cpu.temperature_c, " °C"), "temperature", "Orion’s inside temperature"],
    ["Memory", metricValue(value.ram.percent), "ram", `${bytes(value.ram.used_bytes)} of ${bytes(value.ram.total_bytes)} used`],
    ["Storage", metricValue(value.disk.root.percent), "disk_root", `${bytes(value.disk.root.used_bytes)} of ${bytes(value.disk.root.total_bytes)} used`],
  ];
  return <>
    {!!warnings.size && <div className="system-warning" role="alert"><strong>Orion needs attention</strong><p>{[...warnings].join(" ")}</p></div>}
    <div className="system-tiles">{tiles.map(([label, reading, key, detail]) => <section className="system-tile" key={key}><h2>{label}</h2><strong>{reading}</strong><p>{detail}</p><Sparkline points={metrics.history[key] ?? []} label={label} /><small>Last 15 minutes</small></section>)}</div>
    <section className="settings-card"><h2>Orion’s parts</h2><div className="system-table"><table><thead><tr>{["Part", "Status", "Processor", "Memory"].map(label => <th key={label}>{label}</th>)}</tr></thead><tbody>{value.services.map(service => <tr key={service.name}><th scope="row">{PARTS[service.name] ?? service.name}</th><td>{partState(service.active_state, service.restarts)}</td><td>{metricValue(service.cpu_percent)}</td><td>{bytes(service.memory_bytes)}</td></tr>)}</tbody></table></div></section>
    <div className="system-processes">{(["cpu", "rss"] as const).map(sort => <section className="settings-card" key={sort}><h2>{sort === "cpu" ? "Busiest programs" : "Programs using the most memory"}</h2><div className="system-table"><table><thead><tr><th>Program</th><th>Processor</th><th>Memory</th></tr></thead><tbody>{value.processes[sort].map(process => <tr key={process.pid}><th scope="row">{process.name}</th><td>{metricValue(process.cpu_percent)}</td><td>{bytes(process.rss_bytes)}</td></tr>)}</tbody></table></div></section>)}</div>
  </>;
}
export function System({ connection }: { connection: GatewayConnection | null }) {
  const [metrics, setMetrics] = useState<SystemMetrics | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    setMetrics(null); setError("");
    if (!connection) return;
    return visiblePolling(async signal => {
      try { const data = await getSystemMetrics(connection, signal); if (!signal.aborted) { setMetrics(data); setError(""); } }
      catch (reason) { if (!signal.aborted) setError(reason instanceof Error ? reason.message : "Health readings are not available right now."); }
    });
  }, [connection]);
  return <section className="settings-page system-page" id="workspace"><header className="settings-heading"><p className="eyebrow">HEALTH</p><h1>System</h1><p>{connection ? "How Orion is running right now." : "Connect Orion to see how it is running."}</p></header>{error && <p role="alert" className="settings-error">{error}</p>}{metrics ? <SystemReadings metrics={metrics} /> : connection && !error && <p role="status">Loading…</p>}</section>;
}
