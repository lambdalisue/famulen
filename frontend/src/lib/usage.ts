import { Role, type Usage, type UsageRow, UsageSpan } from "~/gen/famulen/v1/settings_pb";

export type By = "agent" | "model";

export const SPANS = [
  ["week", "週", UsageSpan.WEEK],
  ["month", "月", UsageSpan.MONTH],
  ["year", "年", UsageSpan.YEAR],
] as const;
export type Span = (typeof SPANS)[number][0];

/** The chart colors in app.css; what comes after them is drawn as one. */
const SLOTS = 6;
const OTHER = "";

/** What the owner compares: an agent, or a model. */
export interface Series {
  id: string;
  label: string;
  color: string;
}

export interface Sum {
  calls: number;
  inputTokens: number;
  outputTokens: number;
  yen: number;
}

export function sum(rows: UsageRow[]): Sum {
  return rows.reduce(
    (s, r) => ({
      calls: s.calls + Number(r.calls),
      inputTokens: s.inputTokens + Number(r.inputTokens),
      outputTokens: s.outputTokens + Number(r.outputTokens),
      yen: s.yen + r.yen,
    }),
    { calls: 0, inputTokens: 0, outputTokens: 0, yen: 0 },
  );
}

export function keyOf(row: UsageRow, by: By): string {
  return by === "agent" ? String(row.role) : row.model;
}

function slot(usage: Usage, by: By, key: string): number {
  return by === "agent" ? Number(key) - Role.SECRETARY : usage.models.indexOf(key);
}

/** A key keeps its color whatever period is shown, rather than taking the color of its rank. */
export function colorOf(usage: Usage, by: By, key: string): string {
  const i = slot(usage, by, key);
  return i >= 0 && i < SLOTS ? `var(--usage-${i + 1})` : "var(--usage-other)";
}

/** The keys `rows` hold, in the order of their colors. */
export function keysOf(usage: Usage, by: By, rows: UsageRow[]): string[] {
  return [...new Set(rows.map((r) => keyOf(r, by)))].sort((a, b) => slot(usage, by, a) - slot(usage, by, b));
}

export function seriesOf(usage: Usage, by: By, rows: UsageRow[], label: (key: string) => string): Series[] {
  const series = new Map<string, Series>();
  for (const key of keysOf(usage, by, rows)) {
    const color = colorOf(usage, by, key);
    const id = seriesId(usage, by, key);
    series.set(id, { id, color, label: id === OTHER ? "ほか" : label(key) });
  }
  return [...series.values()];
}

export function seriesId(usage: Usage, by: By, key: string): string {
  return slot(usage, by, key) < SLOTS ? key : OTHER;
}

function parts(day: string): number[] {
  return day.split("-").map(Number);
}

export function md(day: string): string {
  const [, m, d] = parts(day);
  return `${m}/${d}`;
}

/** What a bar stands for, in words. */
export function bucketName(span: Span, from: string): string {
  return span === "year" ? `${parts(from)[1]}月` : md(from);
}

export function periodName(span: Span, usage: Usage): string {
  const [y, m] = parts(usage.from);
  if (span === "year") return `${y}年`;
  if (span === "month") return `${y}年${m}月`;
  return `${y}年${md(usage.from)}〜${md(usage.to)}`;
}

/** The label under a bar; most days of a month have none, so the rest can be read. */
export function tickName(span: Span, from: string): string {
  const [y, m, d] = parts(from);
  if (span === "year") return `${m}月`;
  if (span === "month") return d % 7 === 1 ? md(from) : "";
  return `${md(from)} ${"日月火水木金土"[new Date(Date.UTC(y, m - 1, d)).getUTCDay()]}`;
}

export function shiftDay(day: string, by: number): string {
  const [y, m, d] = parts(day);
  return new Date(Date.UTC(y, m - 1, d + by)).toISOString().slice(0, 10);
}

/** Round values from zero to at least `max`, for an axis. */
export function ticks(max: number, whole: boolean): number[] {
  const rough = Math.max(max, whole ? 1 : 0.01) / 4;
  const power = 10 ** Math.floor(Math.log10(rough));
  const nice = [1, 2, 5, 10].map((n) => n * power).find((n) => n >= rough) ?? rough;
  const step = whole ? Math.max(1, nice) : nice;
  const count = Math.max(1, Math.ceil(max / step - 1e-9));
  return Array.from({ length: count + 1 }, (_, i) => i * step);
}
