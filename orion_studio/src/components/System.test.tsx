import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { System, SystemReadings } from "./System";
import type { SystemMetrics } from "../lib/systemMetrics";
it("renders connection guidance and unavailable readings", () => {
  expect(renderToStaticMarkup(<System connection={null}/>)).toContain("Connect Orion");
  expect(renderToStaticMarkup(<SystemReadings metrics={{ latest: null, history: {} }}/>)).toContain("Waiting");
});
it("renders metrics, histories, services, processes and warnings", () => {
  const disk = { path: "/", percent: 91, used_bytes: 100, total_bytes: 200 };
  const metrics: SystemMetrics = { history: { cpu: [[1, 20], [3, 40]] }, latest: {
    timestamp: 3, cpu: { percent: 40, temperature_c: 76, frequency_mhz: null, load_average: [1, 2, 3], cores: { cpu0: 40 }, throttling: { now: { under_voltage: true }, since_boot: {} } },
    ram: { percent: 50, used_bytes: 100, total_bytes: 200, swap_used_bytes: 0 }, disk: { root: disk, data: disk },
    services: [{ name: "oriond", active_state: "active", restarts: 2, cpu_percent: 1, nr_throttled: 4, memory_bytes: 100 }],
    processes: { cpu: [{ pid: 42, name: "worker", cpu_percent: 30, rss_bytes: 100 }], rss: [] }, hailo_temperature_c: null,
  } };
  const html = renderToStaticMarkup(<SystemReadings metrics={metrics}/>);
  for (const text of ["40.0%", "76.0", "last 15 minutes", "oriond", "worker", "under voltage", "exceeds 75", "exceeds 90%", "CPU cores"]) expect(html).toContain(text);
});
