export interface DiskMetric { path: string; percent: number | null; used_bytes: number | null; total_bytes: number | null }
export interface ProcessMetric { pid: number; name: string; cpu_percent: number | null; rss_bytes: number }
export interface SystemMetrics {
  latest: null | {
    timestamp: number;
    cpu: { percent: number | null; cores: Record<string, number | null>; load_average: number[] | null; frequency_mhz: number | null; temperature_c: number | null; throttling: null | { now: Record<string, boolean>; since_boot: Record<string, boolean> } };
    ram: { percent: number | null; used_bytes: number | null; total_bytes: number | null; swap_used_bytes: number | null };
    disk: { root: DiskMetric; data: DiskMetric };
    services: { name: string; active_state: string | null; restarts: number | null; cpu_percent: number | null; nr_throttled: number | null; memory_bytes: number | null }[];
    processes: { cpu: ProcessMetric[]; rss: ProcessMetric[] };
    hailo_temperature_c: number | null;
  };
  history: Record<string, [number, number | null][]>;
}
export const metricValue = (value: number | null | undefined, unit = "%") => value == null ? "Unavailable" : `${value.toFixed(1)}${unit}`;
export const bytes = (value: number | null) => value == null ? "Unavailable" : `${(value / 1024 ** 3).toFixed(2)} GiB`;
