import { Code, ConnectError } from "@connectrpc/connect";
import { query } from "@solidjs/router";
import type { Field, FieldType, LinkAccountRequest, Record as Rec, RecordItem, RecordKind } from "~/gen/famulen/v1/records_pb";
import type { Workgroup } from "~/gen/famulen/v1/workgroups_pb";
import { api } from "./api";

export type { Field, FieldType, LinkAccountRequest, Rec, RecordItem, RecordKind };

export const OWNER = "owner";
export const SECRETARY = "secretary";
export const WORKER = "worker";

/** The field every kind has: the records it has to do with. */
export const RELATED = "Related";

export const recordsQuery = query(
  async (kind: string, workgroup: string) => (await api.records.listRecords({ kind, workgroup })).records,
  "records",
);

/** One record whole, or null when there is none; a read that fails is thrown, not taken for a record that is gone. */
export const recordQuery = query(async (kind: string, id: string) => {
  try {
    return await api.records.getRecord({ kind, id });
  } catch (e) {
    if (ConnectError.from(e).code === Code.NotFound) return null;
    throw e;
  }
}, "record");

export const kindsQuery = query(async () => (await api.records.listKinds({})).kinds, "kinds");
export const routinesQuery = query(async () => (await api.routines.listRoutines({})).routines, "routines");

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

export function fieldOf(r: { fields: Field[] } | undefined, name: string): Field | undefined {
  return r?.fields.find((f) => same(f.name, name));
}

export function valueOf(r: { fields: Field[] } | undefined, name: string): string {
  return fieldOf(r, name)?.value ?? "";
}

export function valuesOf(r: { fields: Field[] } | undefined, name: string): string[] {
  return fieldOf(r, name)?.values ?? [];
}

export function itemsOf(r: Rec, section: string): RecordItem[] {
  return r.sections.find((s) => same(s.name, section))?.items ?? [];
}

export function sectionOf(r: Rec, section: string): { fields: Field[] } | undefined {
  return r.sections.find((s) => same(s.name, section));
}

/** The kind, id and title a wiki link to a record names. */
export function linkOf(text: string): { kind: string; id: string; title: string } | undefined {
  const m = /^\[\[records\/([^/\]|]+)\/([^/\]|]+?)(?:\.md)?(?:\|([^\]]*))?\]\]$/.exec(text.trim());
  return m ? { kind: m[1], id: m[2], title: m[3] ?? "" } : undefined;
}

export function linkTo(r: { kind: string; id: string; title: string }): string {
  return `[[records/${r.kind}/${r.id}|${r.title.replace(/[[\]|\n]/g, " ").trim()}]]`;
}

const PAGES: { [kind: string]: string } = {
  tasks: "/tasks",
  routines: "/tasks",
  memos: "/memos",
  parties: "/parties",
  holdings: "/things",
  contracts: "/contracts",
  events: "/calendar",
};

/** Where a record opens on the console. */
export function hrefOf(kind: string, id: string): string | undefined {
  const page = PAGES[kind];
  if (!page) return undefined;
  if (kind === "routines") return `${page}?tab=routines&id=${id}`;
  return kind === "events" ? `${page}?record=${id}` : `${page}?id=${id}`;
}

export const KIND_NAMES: { [kind: string]: string } = {
  tasks: "タスク",
  routines: "定期タスク",
  memos: "メモ",
  events: "予定",
  calendars: "カレンダー",
  parties: "相手",
  holdings: "物",
  contracts: "契約",
};

const NAMES: { [name: string]: string } = {
  Status: "状態",
  Assignee: "担当",
  Priority: "優先度",
  Start: "始まり",
  Due: "期日",
  Finished: "済んだ日時",
  "Waiting for": "待っているもの",
  Parent: "親のタスク",
  "Blocked by": "これが済むまで待つ",
  Related: "関わるもの",
  Delegation: "委ね方",
  Runner: "委託先",
  Connection: "つなぎ先",
  Directory: "ディレクトリ",
  Worktree: "新しいワークツリー",
  Base: "もとの版",
  Agent: "エージェント",
  "Agent model": "モデル",
  Permissions: "権限",
  Tools: "渡す道具",
  Files: "渡す手帳のファイル",
  Schedule: "いつ",
  Timezone: "タイムゾーン",
  Until: "終わり",
  Enabled: "動かす",
  "Managed by": "決めた人",
  Skip: "飛ばす回",
  "Skip if": "飛ばす条件",
  When: "条件",
  End: "終わり",
  Calendar: "カレンダー",
  Busy: "時間をふさぐ",
  Response: "出欠",
  Location: "場所",
  Map: "地図",
  Meeting: "会議",
  Repeat: "繰り返し",
  Attendees: "参加者",
  Who: "だれ",
  Email: "メール",
  Organizer: "主催",
  Optional: "任意",
  Exceptions: "ほかと違う回",
  At: "もとの時刻",
  Title: "題名",
  Note: "備考",
  Kind: "種別",
  Aliases: "ほかの名前",
  Birthday: "誕生日",
  "Member of": "属するところ",
  To: "相手",
  Role: "役目",
  Relations: "関係",
  As: "どんな関係か",
  Inverse: "逆から見ると",
  Accounts: "アカウント",
  Service: "サービス",
  Handle: "ID",
  Confirmed: "確かめた",
  How: "どうやって",
  Category: "分類",
  Maker: "メーカー",
  Model: "型",
  Serial: "製造番号",
  Place: "置き場所",
  Acquired: "手に入れた",
  Released: "手放した",
  On: "日付",
  From: "どこから",
  Price: "金額",
  Reason: "理由",
  Deadlines: "期限",
  What: "何の期限か",
  Reminded: "知らせた",
  Provider: "提供元",
  Plan: "プラン",
  Number: "契約番号",
  Started: "始まった日",
  Ended: "終わった日",
  "Auto renew": "自動更新",
  Cycle: "支払いの間隔",
  Payment: "支払い方法",
  Covers: "対象の物",
  Color: "色",
  "Default for": "新しい予定を入れるワークグループ",
};

/** A field's or section's name as the owner reads it. */
export function nameOf(name: string): string {
  return NAMES[name] ?? name;
}

const CHOICES: { [value: string]: string } = {
  pending: "まだ",
  in_progress: "進行中",
  waiting: "待ち",
  done: "済",
  cancelled: "やめた",
  high: "高",
  normal: "ふつう",
  low: "低",
  hold: "仮押さえ",
  tentative: "仮",
  confirmed: "決まった",
  accepted: "出席",
  declined: "欠席",
  needs_action: "未回答",
  person: "人",
  org: "会社・チーム",
  group: "グループ",
  ordered: "注文中",
  owned: "手元にある",
  lent: "貸している",
  released: "手放した",
  bought: "買った",
  gift: "もらった",
  loan: "借りた",
  made: "作った",
  other: "その他",
  sold: "売った",
  given: "あげた",
  returned: "返した",
  discarded: "捨てた",
  lost: "なくした",
  planned: "これから",
  active: "契約中",
  ended: "終わった",
  monthly: "毎月",
  yearly: "毎年",
  once: "一度だけ",
  owner: "あなた",
  secretary: "秘書",
  worker: "委託",
  adroit: "Adroit",
  agent: "手代",
  default: "ふつう",
  read_only: "読むだけ",
  yes: "はい",
  no: "いいえ",
};

export function choiceOf(value: string, aide = "秘書"): string {
  if (value === SECRETARY) return aide;
  return CHOICES[value] ?? value;
}

/** Text without its wiki links' brackets: the titles alone. */
export function plainText(text: string): string {
  return text.replace(/\[\[[^\]|]*\|([^\]]*)\]\]/g, "$1").replace(/\[\[([^\]]*)\]\]/g, "$1");
}

type Bucket = "today" | "week" | "later" | "done";

export const isOver = (r: Rec) => ["done", "cancelled"].includes(valueOf(r, "Status"));

/** Where a task falls by its due date, as of `today` (YYYY-MM-DD). */
export function bucketOf(r: Rec, today: string): Bucket {
  if (isOver(r)) return "done";
  const due = valueOf(r, "Due").slice(0, 10);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(due)) return "later";
  if (due <= today) return "today";
  const days = (Date.parse(`${due}T00:00:00Z`) - Date.parse(`${today}T00:00:00Z`)) / 86_400_000;
  return days < 7 ? "week" : "later";
}

/** Who does a task, in a word. */
export function assigneeOf(r: Rec, aide: string): string {
  const who = valueOf(r, "Assignee");
  return linkOf(who)?.title || choiceOf(who || OWNER, aide);
}

/** The deadlines of a thing or contract that are still to come round, soonest first. */
export function deadlinesOf(r: Rec): { what: string; on: string }[] {
  return itemsOf(r, "Deadlines")
    .map((i) => ({ what: i.text, on: valueOf(i, "On") }))
    .filter((d) => d.on)
    .sort((a, b) => a.on.localeCompare(b.on));
}

/** `3 日後`, `今日`, `5 日過ぎた`. */
export function daysLeft(on: string, today: string): string {
  const days = Math.round((Date.parse(`${on.slice(0, 10)}T00:00:00Z`) - Date.parse(`${today}T00:00:00Z`)) / 86_400_000);
  if (Number.isNaN(days)) return "";
  if (days === 0) return "今日";
  if (days === 1) return "明日";
  return days > 0 ? `${days} 日後` : `${-days} 日過ぎた`;
}

/** Whether every word is somewhere in the record. */
export function matches(r: Rec, words: string): boolean {
  const hay = [
    r.title,
    r.code,
    r.notes,
    ...r.labels,
    ...r.fields.map((f) => f.value),
    ...r.details.flatMap((f) => [f.name, f.value]),
    ...r.sections.flatMap((s) => [...s.fields.map((f) => f.value), ...s.items.flatMap((i) => [i.text, ...i.fields.map((f) => f.value)])]),
  ]
    .join("\n")
    .toLowerCase();
  return words
    .toLowerCase()
    .split(/\s+/)
    .every((w) => hay.includes(w));
}

/** The names of the workgroups a record is in, joined for a sub line. */
export function workgroupNames(workgroups: Workgroup[] | undefined, ids: string[]): string {
  return ids
    .map((id) => workgroups?.find((c) => c.id === id)?.name)
    .filter((n): n is string => !!n)
    .join(" · ");
}

/** A new record goes into the workgroup on screen, unless that is すべて. */
export function newWorkgroups(workgroup: string): string[] {
  return workgroup && workgroup !== "all" ? [workgroup] : [];
}

/** `2026年3月1日`, with its time when it has one. */
export function dateText(text: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}:\d{2}))?$/.exec(text.trim());
  if (!m) return text;
  const year = Number(m[1]) === new Date().getFullYear() ? "" : `${m[1]}年`;
  return `${year}${Number(m[2])}月${Number(m[3])}日${m[4] ? ` ${m[4]}` : ""}`;
}

export function moneyText(text: string): string {
  const m = /^([\d,.]+) ([A-Z]{3})$/.exec(text.trim());
  if (!m) return text;
  try {
    return new Intl.NumberFormat("ja-JP", { style: "currency", currency: m[2] }).format(Number(m[1].replaceAll(",", "")));
  } catch {
    return text;
  }
}
