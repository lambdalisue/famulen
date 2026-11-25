import { query } from "@solidjs/router";
import type { DelegatedTask, WorkerRun, WorkerStep } from "~/gen/famulen/v1/workers_pb";
import { api } from "./api";

export type { DelegatedTask, WorkerRun, WorkerStep };

export const delegatedQuery = query((asked: { workgroup?: string; task?: string } = {}) => api.workers.listDelegated(asked), "delegated");
export const workerSettingsQuery = query(() => api.workers.getWorkerSettings({}), "workerSettings");

export const runOutputQuery = query((id: string) => api.workers.readRun({ id: BigInt(id) }), "runOutput");

export const runHref = (run: WorkerRun) => `/runs/${run.id}`;

const STATES: { [state: string]: string } = {
  queued: "順番待ち",
  starting: "まもなく始まる",
  paused: "一時停止中",
  capped: "今日の上限",
  later: "始める日時を待つ",
  blocked: "先の仕事を待つ",
  unfit: "委ね方が足りない",
  asking: "あなたの承認待ち",
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

const RUN_STATES: { [state: string]: string } = {
  starting: "始めている",
  asking: "承認待ち",
  running: "実行中",
  waiting: "あなたの答え待ち",
  unreachable: "Adroit に届かない",
  ending: "終わりを書いている",
};

/** How a run stands; one that ended says how. */
export const runText = (r: WorkerRun) => (r.state === "ended" ? outcomeText(r.outcome) : (RUN_STATES[r.state] ?? r.state));

/** The steps of a plan with how deep each lies, each below the one it is part of. */
export function outline(steps: WorkerStep[]): { step: WorkerStep; depth: number }[] {
  const ids = new Set(steps.map((s) => s.id));
  const taken = new Set<WorkerStep>();
  const below = (parent: string, depth: number): { step: WorkerStep; depth: number }[] =>
    steps
      .filter((s) => (ids.has(s.parent) ? s.parent : "") === parent && !taken.has(s))
      .flatMap((step) => {
        taken.add(step);
        return [{ step, depth }, ...(step.id ? below(step.id, depth + 1) : [])];
      });
  return below("", 0);
}

/** How long it has gone on, or went on, in words. */
export function lastedText(r: WorkerRun, now: number): string {
  if (!r.startedAt) return "";
  const minutes = Math.max(0, Math.floor(((r.endedAt ? Number(r.endedAt) : now / 1000) - Number(r.startedAt)) / 60));
  return minutes < 60 ? `${minutes} 分` : `${Math.floor(minutes / 60)} 時間 ${minutes % 60} 分`;
}

export const runnerText = (runner: string) => ({ adroit: "Adroit", agent: "手代" })[runner] ?? "委託先なし";

export const isBadOutcome = (outcome: string) => outcome === "failed" || outcome === "refused";

export const isBad = (t: DelegatedTask) => t.state === "unfit" || t.state === "unreachable" || (t.state === "still" && isBadOutcome(t.run?.outcome ?? ""));

type Stage = "owner" | "running" | "soon" | "next" | "held";

/** What a delegated task waits for, in the order the queue shows it: the owner, its end, nothing, a free slot, or something else in its way. */
export const STAGES: [Stage, string][] = [
  ["owner", "あなたを待っている"],
  ["running", "動いている"],
  ["soon", "まもなく始まる"],
  ["next", "順番を待っている"],
  ["held", "始まりを待っている"],
];

export function stageOf(t: DelegatedTask): Stage {
  if (t.state === "running") return "running";
  if (t.state === "starting") return "soon";
  if (["queued", "approved"].includes(t.state)) return "next";
  return ["asking", "waiting", "unfit", "still", "unreachable"].includes(t.state) ? "owner" : "held";
}

/** Its place among those that wait for a free slot; 0 when it does not. */
export const placeOf = (queue: DelegatedTask[] | undefined, task: string) =>
  (queue ?? []).filter((t) => stageOf(t) === "next").findIndex((t) => t.task === task) + 1;
