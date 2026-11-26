import { query } from "@solidjs/router";
import { LimitSeverity, type Model, ModelKind, type UsageLimit, type UsageSpan } from "~/gen/famulen/v1/settings_pb";
import { api } from "./api";
import { dateTimeText } from "./dates";

export const modelsQuery = query(() => api.models.getModels({}), "models");
/** How much of each agent CLI account's limits is used; nothing when it cannot be read, so the page goes on without it. */
export const modelLimitsQuery = query(() => api.models.getModelLimits({}).catch(() => undefined), "model-limits");
export const usageQuery = query((span: UsageSpan, day: string) => api.models.getUsage({ span, day }), "usage");

export const conversationUsageQuery = query(
  (span: UsageSpan, day: string) => api.models.getConversationUsage({ span, day }),
  "conversation-usage",
);

export const KINDS: [ModelKind, string][] = [
  [ModelKind.AGENT_CLI, "エージェント CLI"],
  [ModelKind.LOCAL, "ローカル"],
  [ModelKind.API, "API"],
  [ModelKind.JUDGE, "判定 API"],
  [ModelKind.TRANSCRIBE, "文字起こし"],
];

export function kindLabel(kind: ModelKind): string {
  return KINDS.find(([k]) => k === kind)?.[1] ?? "";
}

export function isMetered(m: Model): boolean {
  return m.inputYen > 0 || m.outputYen > 0 || m.callYen > 0;
}

/** A token count as people read it: 892k, 2.1M. */
export function shortTokens(n: bigint | number): string {
  const v = Number(n);
  if (v >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
  if (v >= 1_000) return `${Math.round(v / 1_000)}k`;
  return String(v);
}

export function yen(n: number): string {
  return `${Math.round(n).toLocaleString("ja-JP")} 円`;
}

/** A limit as the models page draws it, at `nowMs`. */
export type LimitView = {
  label: string;
  /** How much of the bar is filled, from 0 to 100. */
  used: number;
  usedText: string;
  /** When it starts again, or empty when the CLI did not say. */
  note: string;
  tone: "ok" | "warn" | "danger";
};

export function limitView(limit: UsageLimit, nowMs: number, tz: string): LimitView {
  const resets = Number(limit.resetsAtMs);
  const passed = resets > 0 && resets <= nowMs;
  const percent = passed ? 0 : Math.max(0, Math.round(limit.percent));
  const tone = passed
    ? "ok"
    : limit.severity === LimitSeverity.CRITICAL
      ? "danger"
      : limit.severity === LimitSeverity.WARNING
        ? "warn"
        : "ok";
  let note = "";
  if (passed) note = "枠が戻った";
  else if (resets > 0) note = `${dateTimeText(Math.floor(resets / 1000), tz)} に戻る（${untilText(resets - nowMs)}）`;
  return { label: limit.label, used: Math.min(100, percent), usedText: `${percent}% 使った`, note, tone };
}

/** `ms` milliseconds ahead in its two largest units: `あと 4 時間 13 分`. */
export function untilText(ms: number): string {
  const min = Math.floor(ms / 60_000);
  const [d, h, m] = [Math.floor(min / 1440), Math.floor(min / 60) % 24, min % 60];
  if (d > 0) return h > 0 ? `あと ${d} 日 ${h} 時間` : `あと ${d} 日`;
  if (h > 0) return m > 0 ? `あと ${h} 時間 ${m} 分` : `あと ${h} 時間`;
  return m > 0 ? `あと ${m} 分` : "あと 1 分足らず";
}
