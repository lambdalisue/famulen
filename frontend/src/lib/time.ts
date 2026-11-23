/** `11/22 09:30` in `tz`. */
export function shortTime(at: bigint | number, tz: string): string {
  const d = new Date(Number(at) * 1000);
  const parts = new Intl.DateTimeFormat("ja-JP", {
    timeZone: tz || "UTC",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).formatToParts(d);
  const get = (t: string) => parts.find((p) => p.type === t)?.value ?? "";
  return `${get("month")}/${get("day")} ${get("hour")}:${get("minute")}`;
}

/** Today as YYYY-MM-DD in `tz`. */
export function today(tz: string): string {
  return new Intl.DateTimeFormat("sv-SE", { timeZone: tz || "UTC" }).format(new Date());
}
