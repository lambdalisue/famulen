import { query } from "@solidjs/router";
import type { DelegatedTask, WorkerRun } from "~/gen/famulen/v1/workers_pb";
import { api } from "./api";

export type { DelegatedTask, WorkerRun };

export const delegatedQuery = query((asked: { workgroup?: string; task?: string } = {}) => api.workers.listDelegated(asked), "delegated");
export const workerSettingsQuery = query(() => api.workers.getWorkerSettings({}), "workerSettings");

const STATES: { [state: string]: string } = {
  queued: "順番待ち",
  paused: "一時停止中",
  capped: "今日の上限",
  later: "始める日を待つ",
  blocked: "ほかのタスクを待つ",
  unfit: "委ね方が足りない",
  asking: "承認待ち",
  approved: "承認済み・順番待ち",
  retrying: "もう一度始める",
  unreachable: "Adroit に届かない",
  running: "実行中",
  waiting: "あなたの答え待ち",
  still: "止まっている",
};

const OUTCOMES: { [outcome: string]: string } = {
  done: "済んだ",
  failed: "うまくいかなかった",
  refused: "承認されなかった",
  stopped: "止めた",
  interrupted: "途中で切れた",
};

export const outcomeText = (outcome: string) => OUTCOMES[outcome] ?? outcome;

/** How a delegated task stands; one that is still says how its last run ended. */
export function standingText(t: DelegatedTask): string {
  if (t.state === "still" && t.run?.outcome) return outcomeText(t.run.outcome);
  return STATES[t.state] ?? t.state;
}

export const runnerText = (runner: string) => ({ adroit: "Adroit", agent: "手代" })[runner] ?? "委託先なし";

export const isBadOutcome = (outcome: string) => outcome === "failed" || outcome === "refused";

export const isBad = (t: DelegatedTask) => t.state === "unfit" || t.state === "unreachable" || (t.state === "still" && isBadOutcome(t.run?.outcome ?? ""));

type Stage = "owner" | "running" | "next";

/** What a delegated task waits for, in the order the queue shows it: the owner, its end, or its turn. */
export const STAGES: [Stage, string][] = [
  ["owner", "あなたを待っている"],
  ["running", "動いている"],
  ["next", "順番を待っている"],
];

export function stageOf(t: DelegatedTask): Stage {
  if (t.state === "running") return "running";
  return ["asking", "waiting", "unfit", "still", "unreachable"].includes(t.state) ? "owner" : "next";
}
