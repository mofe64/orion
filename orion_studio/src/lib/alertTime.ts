export function alertTime(dueUnix: number, now = Date.now(), locale?: string): string {
  const remaining = dueUnix * 1000 - now;
  const relative = remaining <= 0 ? "due now" : remaining < 60000 ? "in less than a minute" : `in ${Math.ceil(remaining / 60000)} min`;
  const clock = new Date(dueUnix * 1000).toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" });
  return `${relative} (${clock})`;
}
