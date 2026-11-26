import type { RunEntry, RunPart, StepChange, WorkerStep } from "~/gen/famulen/v1/workers_pb";
import * as icons from "./icons";

/** What is being written, by what it is of: `said:<message>`, `input:<call>` and the like. */
export type Parts = Map<string, string>;

export const partKey = (p: { of: string; id: string }) => `${p.of}:${p.id}`;

/** How much of what is being written is held: as much as an entry keeps. */
const MOST_CHARS = 64 * 1024;

/** Take in one part of what is being written: the end of what a tool puts out, the start of the rest. */
export function addPart(parts: Parts, p: RunPart): Parts {
  const next = new Map(parts);
  const key = partKey(p);
  const text = (next.get(key) ?? "") + p.text;
  next.set(key, text.length <= MOST_CHARS ? text : p.of === "output" ? text.slice(-MOST_CHARS) : text.slice(0, MOST_CHARS));
  return next;
}

/** Drop the parts an entry now holds whole. */
export function settle(parts: Parts, e: RunEntry): Parts {
  const done: string[] = [];
  const i = e.item;
  if (i.case === "said") done.push(`said:${i.value.message}`);
  if (i.case === "thought") done.push(`thought:${i.value.message}`);
  if (i.case === "called") done.push(`input:${i.value.call}`);
  if (i.case === "answered") done.push(`output:${i.value.call}`);
  if (!done.some((k) => parts.has(k))) return parts;
  const next = new Map(parts);
  for (const k of done) next.delete(k);
  return next;
}

/** Of the kinds of tool, what a call does to the eye. */
export type ToolKind = "shell" | "test" | "ci" | "read" | "edit" | "search" | "web" | "plan" | "service" | "other";

const TESTS =
  /(^|[\s;&|(])(cargo (nextest|test)|just (ci|test|e2e|ci-macos)|(npm|pnpm|yarn|bun)( run)? (test|e2e|check)|npx (vitest|jest|playwright)|pytest|go test|vitest|jest|playwright test|deno test|mix test|rspec|make (test|check))\b/;
const CI = /(^|[\s;&|(])gh( as -q gh)? (pr checks|run (view|watch|list))\b/;

export function toolKind(tool: string, input: unknown): ToolKind {
  if (tool === "Bash") {
    const command = commandOf(input);
    if (CI.test(command)) return "ci";
    if (TESTS.test(command)) return "test";
    return "shell";
  }
  if (["Read", "NotebookRead"].includes(tool)) return "read";
  if (["Edit", "Write", "MultiEdit", "NotebookEdit"].includes(tool)) return "edit";
  if (["Grep", "Glob", "LS"].includes(tool)) return "search";
  if (["WebSearch", "WebFetch"].includes(tool)) return "web";
  if (tool === "TodoWrite" || tool.endsWith("__timeline_tasks")) return "plan";
  if (tool.startsWith("mcp__")) return "service";
  return "other";
}

export const KINDS: { [K in ToolKind]: { icon: string; text: string } } = {
  shell: { icon: icons.terminal, text: "コマンド" },
  test: { icon: icons.flask, text: "テスト" },
  ci: { icon: icons.gitPullRequest, text: "CI" },
  read: { icon: icons.fileText, text: "読む" },
  edit: { icon: icons.fileDiff, text: "書く" },
  search: { icon: icons.search, text: "探す" },
  web: { icon: icons.world, text: "ウェブ" },
  plan: { icon: icons.listCheck, text: "段取り" },
  service: { icon: icons.plugConnected, text: "つなぎ先" },
  other: { icon: icons.robot, text: "道具" },
};

function field(input: unknown, key: string): string {
  if (typeof input !== "object" || input === null || !(key in input)) return "";
  const value: unknown = Reflect.get(input, key);
  return typeof value === "string" ? value : "";
}

const commandOf = (input: unknown) => field(input, "command");

/** The input of a call, read from its JSON; what cannot be read stays as text. */
export function parseInput(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

/** What a call works on, in a line: the command, the file, the pattern, the page. */
export function summaryOf(tool: string, input: unknown): string {
  for (const key of ["command", "file_path", "notebook_path", "pattern", "url", "query", "path", "description"]) {
    const value = field(input, key);
    if (value) return value.split("\n")[0];
  }
  if (tool.startsWith("mcp__")) return tool.split("__").slice(2).join("__");
  return typeof input === "string" ? input : "";
}

/** How a run of tests or checks came out, as its output says; nothing when it does not. */
export function checksOf(kind: ToolKind, output: string): { passed: number; failed: number } | undefined {
  let passed = 0;
  let failed = 0;
  let any = false;
  if (kind === "ci") {
    for (const line of output.split("\n")) {
      const cells = line.split("\t").map((c) => c.trim());
      if (cells.length < 2) continue;
      if (cells[1] === "pass") passed++;
      if (cells[1] === "fail") failed++;
      any ||= cells[1] === "pass" || cells[1] === "fail";
    }
    return any ? { passed, failed } : undefined;
  }
  if (kind !== "test") return undefined;
  for (const m of output.matchAll(/(\d+) (passed|failed)\b/g)) {
    any = true;
    if (m[2] === "passed") passed += Number(m[1]);
    else failed += Number(m[1]);
  }
  return any ? { passed, failed } : undefined;
}

/** Why a worker's process ended, as the executor tells it. */
const ENDS: { [why: string]: string } = {
  ended: "働き手のプロセスが終わった",
  stopped: "止めたので、働き手のプロセスが終わった",
  took_too_long: "時間の限りを過ぎたので止めた",
  wrote_too_much: "書いた量が限りを超えたので止めた",
  not_kept: "書いたものを残せなくなったので止めた",
  signalled: "マシンの都合で働き手のプロセスが切れた",
};

export type Decision = { allowed: boolean; why: string };

export type Block =
  | { kind: "asked"; seq: bigint; at: bigint; text: string }
  | { kind: "said"; seq: bigint; at: bigint; message: string; text: string; live: boolean; used?: Used }
  | { kind: "thought"; seq: bigint; at: bigint; message: string; text: string; live: boolean }
  | {
      kind: "tool";
      seq: bigint;
      at: bigint;
      call: string;
      tool: string;
      input: unknown;
      /** As it is being written, before it is whole. */
      inputText: string;
      toolKind: ToolKind;
      output?: string;
      ok?: boolean;
      endedAt?: bigint;
      /** What it put out so far while it works. */
      working: string;
      changes: { path: string; diff: string }[];
      decision?: Decision;
      cut: boolean;
    }
  | { kind: "question"; seq: bigint; at: bigint; id: string; tool: string; what: string; decision?: Decision; withdrawn: boolean }
  | { kind: "plan"; seq: bigint; at: bigint; from: string; steps: WorkerStep[] }
  | { kind: "note"; seq: bigint; at: bigint; tone: "warn" | "danger" | "ok" | ""; text: string };

export type Used = { input: bigint; cached: bigint; output: bigint; model: string };

/** The settled entries and what is being written, as the stream shows them, oldest first. */
export function blocks(entries: readonly RunEntry[], parts: Parts): Block[] {
  const out: Block[] = [];
  const calls = new Map<string, Extract<Block, { kind: "tool" }>>();
  const questions = new Map<string, Extract<Block, { kind: "question" }>>();
  const askedOn = new Map<string, Extract<Block, { kind: "tool" }>>();
  const said = new Map<string, Extract<Block, { kind: "said" }>>();
  const used = new Map<string, Used>();
  for (const e of entries) {
    const { seq, at, cut } = e;
    const i = e.item;
    switch (i.case) {
      case "asked":
        out.push({ kind: "asked", seq, at, text: i.value.text });
        break;
      case "said": {
        const block = { kind: "said" as const, seq, at, message: i.value.message, text: i.value.text, live: false };
        said.set(i.value.message, block);
        out.push(block);
        break;
      }
      case "thought":
        out.push({ kind: "thought", seq, at, message: i.value.message, text: i.value.text, live: false });
        break;
      case "called": {
        const input = parseInput(i.value.input);
        const block = {
          kind: "tool" as const,
          seq,
          at,
          call: i.value.call,
          tool: i.value.tool,
          input,
          inputText: "",
          toolKind: toolKind(i.value.tool, input),
          working: "",
          changes: [],
          cut,
        };
        calls.set(i.value.call, block);
        out.push(block);
        break;
      }
      case "answered": {
        const call = calls.get(i.value.call);
        if (call) {
          call.output = i.value.output;
          call.ok = i.value.ok;
          call.endedAt = at;
          call.cut ||= cut;
        }
        break;
      }
      case "changed":
        calls.get(i.value.call)?.changes.push({ path: i.value.path, diff: i.value.diff });
        break;
      case "question": {
        // What an agent asks to use is shown on its call, when there is one.
        const call = [...calls.values()].reverse().find((c) => c.tool === i.value.tool && !c.decision && c.ok === undefined);
        const block = { kind: "question" as const, seq, at, id: i.value.id, tool: i.value.tool, what: i.value.what, withdrawn: false };
        questions.set(i.value.id, block);
        if (call) askedOn.set(i.value.id, call);
        else out.push(block);
        break;
      }
      case "decided": {
        const q = questions.get(i.value.id);
        const decision = { allowed: i.value.allowed, why: i.value.why };
        if (q) q.decision = decision;
        const call = askedOn.get(i.value.id);
        if (call) call.decision = decision;
        break;
      }
      case "withdrawn": {
        const q = questions.get(i.value);
        if (q) q.withdrawn = true;
        break;
      }
      case "plan":
        out.push({ kind: "plan", seq, at, from: i.value.from, steps: i.value.steps });
        break;
      case "used":
        used.set(i.value.message, { input: i.value.input, cached: i.value.cached, output: i.value.output, model: i.value.model });
        break;
      case "warned":
        out.push({ kind: "note", seq, at, tone: "warn", text: i.value });
        break;
      case "turnEnded":
        out.push({ kind: "note", seq, at, tone: i.value.ok ? "ok" : "danger", text: i.value.ok ? "ターンを終えた" : i.value.text || "ターンがうまく終わらなかった" });
        break;
      case "ended":
        out.push({ kind: "note", seq, at, tone: i.value === "ended" ? "" : "warn", text: ENDS[i.value] ?? `働き手のプロセスが終わった（${i.value}）` });
        break;
      case "forgotten":
        out.push({ kind: "note", seq, at, tone: "", text: "日が経ったので、予定と結果のほかの出力は消した" });
        break;
      case "full":
        out.push({ kind: "note", seq, at, tone: "warn", text: "残せる量を超えた。ここからの出力は短く切って残している" });
        break;
    }
  }
  for (const [message, u] of used) {
    const block = said.get(message);
    if (block) block.used = u;
  }
  // What is being written comes last, after all that settled.
  const live = BigInt(Number.MAX_SAFE_INTEGER);
  for (const [key, text] of parts) {
    const [of, ...rest] = key.split(":");
    const id = rest.join(":");
    if (of === "said") out.push({ kind: "said", seq: live, at: 0n, message: id, text, live: true });
    if (of === "thought") out.push({ kind: "thought", seq: live, at: 0n, message: id, text, live: true });
    if (of === "output") {
      const call = calls.get(id);
      if (call) call.working += text;
    }
    if (of === "input" && !calls.has(id)) {
      out.push({
        kind: "tool",
        seq: live,
        at: 0n,
        call: id,
        tool: "",
        input: undefined,
        inputText: text,
        toolKind: "other",
        working: "",
        changes: [],
        cut: false,
      });
    }
  }
  return out;
}

/** A key that tells a block from the others, the same while it grows. */
export function blockKey(b: Block): string {
  switch (b.kind) {
    case "said":
      return b.live ? `live-said:${b.message}` : `${b.seq}`;
    case "thought":
      return b.live ? `live-thought:${b.message}` : `${b.seq}`;
    case "tool":
      return b.tool ? `${b.seq}` : `live-call:${b.call}`;
    default:
      return `${b.seq}`;
  }
}

/** When a step was under way, from what changed in the plan: from its start to its end, or on. */
export type Span = { step: WorkerStep; depth: number; from?: bigint; to?: bigint; seq?: bigint; changes: StepChange[] };

export function spans(rows: { step: WorkerStep; depth: number }[], changes: readonly StepChange[]): Span[] {
  return rows.map(({ step, depth }) => {
    const own = changes.filter((c) => c.step === step.id);
    const started = own.find((c) => c.change === "status" && c.to === "in_progress") ?? own.find((c) => c.change === "added");
    const ended = [...own].reverse().find((c) => c.change === "status" && (c.to === "completed" || c.to === "blocked"));
    const from = own.find((c) => c.change === "status" && c.to === "in_progress")?.at;
    return {
      step,
      depth,
      from: from ?? (ended ? started?.at : undefined),
      to: ended && step.status !== "in_progress" ? ended.at : undefined,
      seq: started?.seq,
      changes: own,
    };
  });
}

/** `12.3k` of a count of tokens. */
export function tokensText(n: bigint | number): string {
  const v = Number(n);
  if (v < 1000) return String(v);
  if (v < 1_000_000) return `${(v / 1000).toFixed(v < 10_000 ? 1 : 0)}k`;
  return `${(v / 1_000_000).toFixed(1)}M`;
}

/** `3.2 秒`, `4 分 05 秒` of a length of time in milliseconds. */
export function durationText(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (ms < 10_000) return `${(Math.max(0, ms) / 1000).toFixed(1)} 秒`;
  if (s < 60) return `${s} 秒`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} 分 ${String(s % 60).padStart(2, "0")} 秒`;
  return `${Math.floor(m / 60)} 時間 ${String(m % 60).padStart(2, "0")} 分`;
}

/** The sums of what each message of a run used. */
export function usedSum(entries: readonly RunEntry[]): Used {
  const last = new Map<string, Used>();
  for (const e of entries) {
    if (e.item.case === "used") {
      const u = e.item.value;
      last.set(u.message, { input: u.input, cached: u.cached, output: u.output, model: u.model });
    }
  }
  const sum: Used = { input: 0n, cached: 0n, output: 0n, model: "" };
  for (const u of last.values()) {
    sum.input += u.input;
    sum.cached += u.cached;
    sum.output += u.output;
    sum.model ||= u.model;
  }
  return sum;
}

export const PLAN_FROM: { [from: string]: string } = {
  timeline: "Famulen の道具で書いた段取り",
  adroit: "Adroit のタイムライン",
  notes: "覚え書きから自動で読んだ段取り",
};
