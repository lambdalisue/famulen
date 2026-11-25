import { query } from "@solidjs/router";
import { type Model, ModelKind, type UsageSpan } from "~/gen/famulen/v1/settings_pb";
import { api } from "./api";

export const modelsQuery = query(() => api.models.getModels({}), "models");
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
