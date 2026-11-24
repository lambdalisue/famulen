import { Code, ConnectError } from "@connectrpc/connect";
import { query } from "@solidjs/router";
import type { Field, FieldType, LinkAccountRequest, Record as Rec, RecordItem, RecordKind } from "~/gen/famulen/v1/records_pb";
import type { Workgroup } from "~/gen/famulen/v1/workgroups_pb";
import { api } from "./api";
import { daysBetween } from "./dates";
import { hasWords } from "./words";

export type { Field, FieldType, LinkAccountRequest, Rec, RecordItem, RecordKind };

export const OWNER = "owner";
export const SECRETARY = "secretary";
export const WORKER = "worker";

/** What a field holds, as it is saved. */
export type FieldValue = { value: string; values: string[] };
/** A field with what it holds, whether it came from the API or is being put together. */
export type FieldLike = FieldValue & { name: string };
/** What a field may hold. */
export type FieldSpec = Pick<FieldType, "name" | "type" | "choices" | "of">;
/** An item of a section, as it is saved. */
export type ItemLike = { text: string; fields: FieldLike[] };

export const plainField = (f: FieldLike): FieldLike => ({ name: f.name, value: f.value, values: f.values });
export const plainItem = (i: ItemLike): ItemLike => ({ text: i.text, fields: i.fields.map(plainField) });
export const isEmpty = (v: FieldValue) => !v.value && v.values.length === 0;

/** Set a field among others, or take it out when it is left empty. */
export function withField(fields: readonly FieldLike[], name: string, v: FieldValue): FieldLike[] {
  const left = fields.filter((f) => !same(f.name, name)).map(plainField);
  return isEmpty(v) ? left : [...left, { name, ...v }];
}

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

/** Names of fields and sections are read whatever their case. */
export const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

export function fieldOf<T extends FieldLike>(r: { fields: T[] } | undefined, name: string): T | undefined {
  return r?.fields.find((f) => same(f.name, name));
}

export function valueOf(r: { fields: FieldLike[] } | undefined, name: string): string {
  return fieldOf(r, name)?.value ?? "";
}

export function valuesOf(r: { fields: FieldLike[] } | undefined, name: string): string[] {
  return fieldOf(r, name)?.values ?? [];
}

export function itemsOf(r: Rec, section: string): RecordItem[] {
  return r.sections.find((s) => same(s.name, section))?.items ?? [];
}

/** A section of a record, by its name. */
export function sectionIn(r: Rec, name: string) {
  return r.sections.find((s) => same(s.name, name));
}

export function sectionOf(r: Rec, section: string): { fields: FieldLike[] } | undefined {
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
  return daysBetween(today, due) < 7 ? "week" : "later";
}

/** Who does a task, in a word. */
export function assigneeOf(r: Rec, aide: string): string {
  const who = valueOf(r, "Assignee");
  return linkOf(who)?.title || choiceOf(who || OWNER, aide);
}

/** Whether a thing was let go, or a contract is over: its deadlines no longer matter. */
export const isPast = (r: Rec) => ["released", "ended"].includes(valueOf(r, "Status"));

/** Text of Markdown on one line, as a list shows it. */
export function excerptOf(markdown: string, length = 160): string {
  return plainText(markdown)
    .replace(/[#>*`|_~-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, length);
}

/** Every deadline of a thing or contract, soonest first. */
export function deadlinesOf(r: Rec): { what: string; on: string }[] {
  return itemsOf(r, "Deadlines")
    .map((i) => ({ what: i.text, on: valueOf(i, "On") }))
    .filter((d) => d.on)
    .sort((a, b) => a.on.localeCompare(b.on));
}

/** Whether every word is somewhere in the record. */
export function matches(r: Rec, words: string): boolean {
  const said = [
    r.title,
    r.code,
    r.notes,
    ...r.labels,
    ...r.fields.map((f) => f.value),
    ...r.details.flatMap((f) => [f.name, f.value]),
    ...r.sections.flatMap((s) => [...s.fields.map((f) => f.value), ...s.items.flatMap((i) => [i.text, ...i.fields.map((f) => f.value)])]),
  ];
  return hasWords(said.join("\n"), words);
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

export function moneyText(text: string): string {
  const m = /^([\d,.]+) ([A-Z]{3})$/.exec(text.trim());
  if (!m) return text;
  try {
    return new Intl.NumberFormat("ja-JP", { style: "currency", currency: m[2] }).format(Number(m[1].replaceAll(",", "")));
  } catch {
    return text;
  }
}
