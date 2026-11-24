import type { Connection } from "~/gen/famulen/v1/services_pb";

/** What a held call is, as the owner reads it. */
export type Asked = {
  /** What it does, as a sentence: `セッションを始める`. */
  does: string;
  /** Where it does it: the service and the account. */
  where: string;
};

/** One thing a call is given. */
export type Given = { name: string; text: string; lines: boolean; code: boolean; rows: Given[] };

/** Calls that read wrong when put together from their words. */
const WHOLE: { [call: string]: string } = {
  Bash: "コマンドを実行する",
  BashOutput: "コマンドの出力を読む",
  KillShell: "動いているコマンドを止める",
  Read: "ファイルを読む",
  Write: "ファイルを書く",
  Edit: "ファイルを直す",
  MultiEdit: "ファイルを直す",
  NotebookEdit: "ノートブックを直す",
  Glob: "ファイルを探す",
  Grep: "ファイルの中を探す",
  WebFetch: "ウェブの頁を読む",
  WebSearch: "ウェブを探す",
  Task: "エージェントを始める",
  Agent: "エージェントを始める",
  "adroit.send_to_session": "セッションに伝える",
  "adroit.put_question": "セッションからの質問に答える",
  "adroit.answer_permission": "セッションの許可の求めに答える",
  "slack.open_dm": "DM を開く",
  "slack.set_status": "考え中の表示を変える",
  "google.send_message": "メールを送る",
  "google.search_messages": "メールを探す",
  "google.get_message": "メールを読む",
  "google.respond_to_event": "予定の出欠を答える",
  "google.watch_mailbox": "新しいメールを知らせてもらう",
  "google.watch_events": "予定の変更を知らせてもらう",
  "google.stop_watching": "変更の知らせを止める",
  "bluesky.create_post": "投稿する",
  "kingoftime.record_time": "打刻する",
  "definitions.change_definition": "サービス定義を変える",
};

const VERBS: { [verb: string]: string } = {
  accept: "受ける",
  add: "足す",
  answer: "答える",
  append: "書き足す",
  approve: "承認する",
  archive: "しまう",
  assign: "割り当てる",
  cancel: "取り消す",
  change: "変える",
  clear: "空にする",
  close: "閉じる",
  comment: "コメントする",
  complete: "済ませる",
  copy: "写す",
  create: "作る",
  delete: "消す",
  disable: "切る",
  download: "ダウンロードする",
  edit: "直す",
  enable: "入れる",
  end: "終える",
  export: "書き出す",
  fetch: "読む",
  forward: "転送する",
  generate: "作る",
  get: "読む",
  import: "取り込む",
  insert: "足す",
  interrupt: "止める",
  invite: "招く",
  list: "一覧で読む",
  mark: "印を付ける",
  merge: "マージする",
  move: "動かす",
  open: "開く",
  pause: "一時停止する",
  post: "投稿する",
  publish: "公開する",
  put: "入れる",
  query: "調べる",
  read: "読む",
  remove: "外す",
  rename: "名前を変える",
  reply: "返事をする",
  request: "頼む",
  resolve: "解決にする",
  restore: "戻す",
  resume: "続ける",
  revoke: "取り消す",
  run: "動かす",
  save: "保存する",
  schedule: "予約する",
  search: "探す",
  send: "送る",
  set: "決める",
  share: "共有する",
  start: "始める",
  stop: "止める",
  submit: "出す",
  trash: "ごみ箱に入れる",
  unregister: "登録から外す",
  update: "変える",
  upload: "アップロードする",
  write: "書く",
};

/** Verbs that take に rather than を. */
const TO = new Set(["answer", "comment", "mark", "reply"]);

const NOUNS: { [noun: string]: string } = {
  account: "アカウント",
  block: "ブロック",
  board: "ボード",
  branch: "ブランチ",
  bucket: "バケット",
  calendar: "カレンダー",
  channel: "チャンネル",
  collection: "コレクション",
  comment: "コメント",
  connection: "つなぎ先",
  contact: "連絡先",
  database: "データベース",
  definition: "サービス定義",
  document: "文書",
  draft: "下書き",
  email: "メール",
  event: "予定",
  feed: "フィード",
  file: "ファイル",
  folder: "フォルダー",
  form: "フォーム",
  issue: "Issue",
  item: "項目",
  label: "ラベル",
  list: "リスト",
  lookout: "見張り",
  mail: "メール",
  member: "メンバー",
  memory: "記憶",
  message: "メッセージ",
  model: "モデル",
  note: "メモ",
  page: "頁",
  permission: "権限",
  post: "投稿",
  project: "プロジェクト",
  pull_request: "プルリクエスト",
  question: "質問",
  reaction: "リアクション",
  record: "記録",
  release: "リリース",
  repository: "リポジトリ",
  review: "レビュー",
  routine: "定期タスク",
  row: "行",
  secret: "秘密",
  session: "セッション",
  space: "スペース",
  table: "表",
  task: "タスク",
  team: "チーム",
  terminal: "端末",
  thread: "スレッド",
  user: "ユーザー",
  value: "値",
  view: "ビュー",
  workgroup: "ワークグループ",
  workspace: "ワークスペース",
};

function noun(words: string[]): string {
  const said = words.join("_").toLowerCase();
  const one = said.replace(/ies$/, "y").replace(/s$/, "");
  return NOUNS[said] ?? NOUNS[one] ?? `「${words.join(" ")}」`;
}

/** A tool's name, put into words as far as they are known. */
function inWords(tool: string, kind: string): string {
  const words = tool
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .split(/[_\-.\s]+/)
    .filter(Boolean);
  if (words.length > 1 && words[0].toLowerCase() === kind) words.shift();
  const [first, ...rest] = words;
  const verb = VERBS[first?.toLowerCase() ?? ""];
  if (!verb) return `「${words.join(" ")}」を使う`;
  if (rest.length === 0) return verb;
  return `${noun(rest)}${TO.has(first.toLowerCase()) ? "に" : "を"}${verb}`;
}

const OWN: { [connection: string]: string } = { famulen: "Famulen", vault: "金庫", definitions: "サービス定義" };

/** What a held call does and where, from how it is named to the model. */
export function askedOf(tool: string, connections: readonly Connection[]): Asked {
  const m = /^mcp__(.+?)__(.+)$/.exec(tool) ?? /^(vault|definitions)\.(.+)$/.exec(tool);
  if (!m) return { does: WHOLE[tool] ?? inWords(tool, ""), where: "この機械" };
  const [, id, name] = m;
  const there = connections.find((c) => c.id === id);
  const kind = there?.kind || id.replace(/-\d+$/, "");
  const service = OWN[id] ?? (there?.name || id);
  return {
    does: WHOLE[`${kind}.${name}`] ?? inWords(name, kind),
    where: [service, there?.account].filter((s) => s && s !== service).length > 0 ? `${service} · ${there?.account}` : service,
  };
}

const NAMES: { [name: string]: string } = {
  agent: "エージェント",
  attendees: "参加者",
  base: "もとの版",
  bcc: "BCC",
  blocks: "ブロック",
  body: "本文",
  calendar: "カレンダー",
  cc: "CC",
  channel: "チャンネル",
  command: "コマンド",
  content: "中身",
  cwd: "ディレクトリ",
  date: "日付",
  definition: "サービス定義",
  description: "説明",
  dir: "ディレクトリ",
  email: "メール",
  emoji: "絵文字",
  end: "終わり",
  event: "予定",
  file_path: "ファイル",
  id: "ID",
  input: "中身",
  kind: "種類",
  labels: "ラベル",
  link: "リンク",
  location: "場所",
  message: "メッセージ",
  model: "モデル",
  name: "名前",
  new_string: "新しい文",
  old_string: "もとの文",
  owner: "持ち主",
  path: "パス",
  permissions: "権限",
  priority: "優先度",
  project: "プロジェクト",
  prompt: "伝えること",
  query: "探す言葉",
  raw: "送るメール",
  repo: "リポジトリ",
  response: "答え",
  send_updates: "知らせる相手",
  session: "セッション",
  start: "始まり",
  status: "状態",
  subject: "件名",
  summary: "題名",
  text: "本文",
  thread: "スレッド",
  timeout: "待つ長さ（ミリ秒）",
  title: "題名",
  to: "宛先",
  tool: "道具",
  truncated: "途中で切れている",
  url: "URL",
  user: "ユーザー",
  why: "わけ",
  workgroup: "ワークグループ",
  worktree: "新しいワークツリー",
};

const CODE = new Set(["command", "path", "file_path", "dir", "cwd", "old_string", "new_string"]);

/** An email as Gmail is given one, in the text it is. */
function letter(raw: string): string {
  try {
    const bytes = Uint8Array.from(atob(raw.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0));
    const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return /^[\w-]+: /m.test(text) ? text : raw;
  } catch {
    return raw;
  }
}

function given(key: string, value: unknown): Given {
  const name = NAMES[key] ?? key.replaceAll("_", " ");
  const said = (text: string) => ({ name, text, lines: text.includes("\n") || [...text].length > 60, code: CODE.has(key), rows: [] });
  if (typeof value === "string") return said(key === "raw" ? letter(value) : value);
  if (typeof value === "boolean") return said(value ? "はい" : "いいえ");
  if (value === null || value === undefined) return said("（なし）");
  if (typeof value !== "object") return said(String(value));
  if (Array.isArray(value)) {
    if (value.every((v) => typeof v !== "object" || v === null)) return said(value.map(String).join("、"));
    return { ...said(""), rows: value.map((v, i) => ({ ...given("", v), name: String(i + 1) })) };
  }
  return { ...said(""), rows: Object.entries(value).map(([k, v]) => given(k, v)) };
}

/** Everything a call is given, one row for each; undefined when it is not an object. */
export function givenOf(input: string): Given[] | undefined {
  try {
    const value: unknown = JSON.parse(input);
    if (typeof value !== "object" || value === null || Array.isArray(value)) return undefined;
    return Object.entries(value)
      .filter(([, v]) => v !== "" && !(Array.isArray(v) && v.length === 0))
      .map(([k, v]) => given(k, v));
  } catch {
    return undefined;
  }
}
