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
export function SystemReadings({ metrics }: { metrics: SystemMetrics }) {
  const value = metrics.latest;
  if (!value) return <p role="status">Waiting for the first system sample…</p>;
  const warnings = [
    ...Object.entries(value.cpu.throttling?.now ?? {}).filter(([, active]) => active).map(([name]) => name.replaceAll("_", " ")),
    ...(Object.values(value.cpu.throttling?.since_boot ?? {}).some(Boolean) ? ["Throttling occurred since boot"] : []),
    ...(value.cpu.temperature_c != null && value.cpu.temperature_c > 75 ? ["CPU temperature exceeds 75 °C"] : []),
    ...Object.values(value.disk).filter(disk => disk.percent != null && disk.percent > 90).map(disk => `${disk.path} exceeds 90% disk use`),
  ];
  const tiles = [
    ["CPU", metricValue(value.cpu.percent), "cpu", `${metricValue(value.cpu.frequency_mhz, " MHz")} · Load ${value.cpu.load_average?.map(v => v.toFixed(2)).join(" / ") ?? "unavailable"}`],
    ["Temperature", metricValue(value.cpu.temperature_c, " °C"), "temperature", `Hailo: ${metricValue(value.hailo_temperature_c, " °C")}`],
    ["RAM", metricValue(value.ram.percent), "ram", `${bytes(value.ram.used_bytes)} of ${bytes(value.ram.total_bytes)} · Swap ${bytes(value.ram.swap_used_bytes)}`],
    ["Disk /", metricValue(value.disk.root.percent), "disk_root", `${bytes(value.disk.root.used_bytes)} of ${bytes(value.disk.root.total_bytes)}`],
    ["Orion data", metricValue(value.disk.data.percent), "disk_data", value.disk.data.path],
  ];
  return <>
    {!!warnings.length && <div className="system-warning" role="alert"><strong>System needs attention</strong><p>{warnings.join(" · ")}</p></div>}
    <div className="system-tiles">{tiles.map(([label, reading, key, detail]) => <section className="system-tile" key={key}><h2>{label}</h2><strong>{reading}</strong><p>{detail}</p><Sparkline points={metrics.history[key] ?? []} label={label} /><small>15 minutes</small></section>)}</div>
    <details className="system-cores"><summary>CPU cores</summary><dl>{Object.entries(value.cpu.cores).map(([core, percent]) => <div key={core}><dt>{core}</dt><dd>{metricValue(percent)}</dd></div>)}</dl></details>
    <section className="settings-card"><h2>Services</h2><div className="system-table"><table><thead><tr>{["Service", "State", "Restarts", "CPU", "Throttled periods", "Memory"].map(label => <th key={label}>{label}</th>)}</tr></thead><tbody>{value.services.map(service => <tr key={service.name}><th scope="row">{service.name}</th><td>{service.active_state ?? "Unavailable"}</td><td>{service.restarts ?? "—"}</td><td>{metricValue(service.cpu_percent)}</td><td>{service.nr_throttled ?? "—"}</td><td>{bytes(service.memory_bytes)}</td></tr>)}</tbody></table></div></section>
    <div className="system-processes">{(["cpu", "rss"] as const).map(sort => <section className="settings-card" key={sort}><h2>Top processes by {sort === "cpu" ? "CPU" : "memory"}</h2><div className="system-table"><table><thead><tr><th>Process</th><th>PID</th><th>CPU</th><th>Memory</th></tr></thead><tbody>{value.processes[sort].map(process => <tr key={process.pid}><th scope="row">{process.name}</th><td>{process.pid}</td><td>{metricValue(process.cpu_percent)}</td><td>{bytes(process.rss_bytes)}</td></tr>)}</tbody></table></div></section>)}</div>
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
      catch (reason) { if (!signal.aborted) setError(reason instanceof Error ? reason.message : "System readings unavailable."); }
    });
  }, [connection]);
  return <section className="settings-page system-page"><header className="settings-heading"><p className="eyebrow">PI HEALTH</p><h1>System</h1><p>{connection ? "Live readings · every 2 seconds · 15 minutes in memory" : "Connect Orion to view system readings."}</p></header>{error && <p role="alert" className="settings-error">{error}</p>}{metrics ? <SystemReadings metrics={metrics} /> : connection && !error && <p role="status">Loading system readings…</p>}</section>;
}
