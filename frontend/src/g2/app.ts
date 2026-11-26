/**
 * The secretary on the Even G2: the conversation the glasses open on, a menu
 * of what to look at on the spot, and talking by voice.
 *
 * Tap talks, a swipe goes up and down, a long press opens the menu, a double
 * tap goes back, and on the conversation asks whether to leave.
 */

import { ApprovalAnswer, type Approval, type ApprovalGiven, type ConsoleEvent, type ConsoleMessage, type Conversation, Speaker } from "~/gen/famulen/v1/console_pb";
import type { CalendarEvent } from "~/gen/famulen/v1/calendar_pb";
import type { Record as Rec } from "~/gen/famulen/v1/records_pb";
import { api } from "~/lib/api";
import { addDays, clock, dayIn, hm, localDate, WEEKDAYS, weekdayIndex } from "~/lib/dates";
import { excerptOf, isOver, OWNER, plainText, SECRETARY, valueOf, WORKER } from "~/lib/records";
import type { Glasses, Input } from "./even";
import type { IconName } from "./icons";
import { type Level, type Page, Painter } from "./page";
import {
  CARD_LINES,
  CARD_WIDTH,
  type Card,
  type Draw,
  dots,
  draw,
  HELD_WIDTH,
  heldLines,
  heldScreen,
  type Line,
  listenScreen,
  listScreen,
  type MenuItem,
  menuScreen,
  ROW_WIDTH,
  type Row,
  TALK_LINES,
  TALK_WIDTH,
  type Top,
  talkScreen,
} from "./screens";
import { clip, EM, linesIn, measure, pad, wrap } from "./text";
import { Listening, NOTHING_MS, RATE } from "./voice";

const ALL = "all";
/** How long the dots wait for the secretary when nothing tells how long it works, as the console waits. */
const THINKING_SECONDS = 30 * 60;
/** Where what the owner says on the glasses tells the secretary it was said. */
const VIEW = "Even G2";

type Screen =
  | { name: "talk"; page: number | undefined }
  | { name: "menu"; selected: number }
  | { name: "talks"; selected: number }
  | { name: "today"; selected: number }
  | { name: "tasks"; selected: number }
  | { name: "task"; id: string; selected: number }
  | { name: "confirms"; selected: number }
  | { name: "confirm"; id: bigint; selected: number; page: number; replies: string[] }
  | { name: "memos"; selected: number }
  | { name: "memo"; id: string; page: number }
  | { name: "listen"; about?: string; heard: string; back: Screen };

type Data = {
  aide: string;
  tz: string;
  workgroup: string;
  talks: Conversation[];
  messages: ConsoleMessage[];
  approvals: Approval[];
  events: CalendarEvent[];
  tasks: Rec[];
  memos: Rec[];
};

const MENU: { icon: IconName; label: string; screen: Screen["name"] }[] = [
  { icon: "talk", label: "会話", screen: "talks" },
  { icon: "calendar", label: "今日の予定", screen: "today" },
  { icon: "tasks", label: "タスク", screen: "tasks" },
  { icon: "confirm", label: "確認", screen: "confirms" },
  { icon: "memos", label: "メモ", screen: "memos" },
];

/**
 * Words that answer the one approval on screen when they are all that is said,
 * or all that is said before a pause, as in 「送って、あと…」. Words that only
 * begin a sentence, such as お願い in お願いがある, answer nothing.
 */
const YES = /^(はい|うん|ええ|お願いします|おねがいします|お願い|送って|送ってください|いいよ|いいです|オーケー|オッケー|ok|許可|許可する)(?:[、。,.!！？?\s]|$)/i;
/** Words after a yes that take it back. */
const TAKEN_BACK = /やめ|止め|だめ|ダメ|待って|まって|違う|ちがう|キャンセル|いや|送らない|しないで/;
const NO = /^(いいえ|やめて|やめる|やめておいて|だめ|ダメ|送らないで|キャンセル|中止)(?:[、。,.!！？?\s]|$)/;

/** What a spoken answer to an approval means, if it is one. */
export function spokenAnswer(text: string): ApprovalAnswer | undefined {
  const said = text.trim();
  if (NO.test(said)) return ApprovalAnswer.DENY;
  // 「はい、やっぱりやめて」 takes the yes back; what is unsure is left to the secretary.
  if (YES.test(said) && !TAKEN_BACK.test(said.replace(YES, ""))) return ApprovalAnswer.ALLOW;
  return undefined;
}

/** What a call failed with, in words for the glasses. */
function why(e: unknown): string {
  const text = e instanceof Error ? e.message : String(e);
  if (/no model is assigned to 声の聞き取り/.test(text)) return "声の聞き取りのモデルが選ばれていない。設定で選ぶ";
  if (/声の聞き取り reached today's cap/.test(text)) return "声の聞き取りが今日の上限に達した";
  return clip(text.replace(/^\[\w+\]\s*/, ""), 900);
}

/** An approval's whole text, as the owner must read all of it before answering. */
export function approvalText(a: Approval): string {
  const rows = (given: ApprovalGiven[], depth: number): string[] =>
    given.flatMap((g) => {
      const name = g.name ? `${g.name}：` : "";
      if (g.rows.length) return [`${"  ".repeat(depth)}${g.name}`, ...rows(g.rows, depth + 1)];
      const indent = "  ".repeat(depth);
      return g.text.includes("\n") || measure(g.text) > 20 * EM ? [`${indent}${name}`, ...g.text.split("\n").map((l) => `${indent}${l}`)] : [`${indent}${name}${g.text}`];
    });
  const body = a.given.length ? rows(a.given, 0) : [a.input];
  return [a.summary, ...body].filter((t) => t.trim()).join("\n");
}

export class G2 {
  #glasses: Glasses;
  #painter: Painter;
  #draw: Draw;
  #screen: Screen = { name: "talk", page: undefined };
  #data: Data = { aide: "秘書", tz: "Asia/Tokyo", workgroup: "すべて", talks: [], messages: [], approvals: [], events: [], tasks: [], memos: [] };
  #talk: bigint | undefined;
  /** The next thing said begins a conversation. */
  #fresh = false;
  #thinking = false;
  #notice = "";
  #step = 0;
  #listening: Listening | undefined;
  #hearing = false;
  #stop = new AbortController();
  #timers: ReturnType<typeof setInterval>[] = [];
  /** A hearing of what is said so far is under way. */
  #interim = false;
  /** What was said is being heard and sent. */
  #sending = false;
  /** What the owner said from here waits for the secretary's answer. */
  #awaiting = false;
  #thinkingUntil: ReturnType<typeof setTimeout> | undefined;
  #loads = 0;
  /** Gives up listening when the microphone sends nothing. */
  #silentUntil: ReturnType<typeof setTimeout> | undefined;
  /** Told of what happened, for the phone's screen. */
  onState: (state: string) => void = () => {};

  constructor(glasses: Glasses, art: Draw = draw) {
    this.#glasses = glasses;
    this.#painter = new Painter(glasses);
    this.#draw = art;
  }

  async start() {
    this.#glasses.onInput((input) => void this.#on(input).catch((e) => this.#fail(e)));
    // Set before anything is awaited, so that stopping while loading stops them too.
    this.#timers.push(setInterval(() => {
      if (this.#hearing || this.#thinking) {
        this.#step++;
        void this.#paint();
      }
    }, 400));
    this.#timers.push(setInterval(() => void this.#paint(), 30_000));
    await this.#load();
    if (this.#stop.signal.aborted) return;
    await this.#paint();
    void this.#follow();
  }

  #fail(e: unknown) {
    this.#notice = why(e);
    this.onState(this.#notice);
    void this.#paint();
  }

  /** Show or stop the dots of the secretary thinking, for at most `seconds`. */
  #think(on: boolean, seconds: number) {
    clearTimeout(this.#thinkingUntil);
    this.#thinking = on;
    if (!on) this.#awaiting = false;
    else
      this.#thinkingUntil = setTimeout(() => {
        this.#think(false, 0);
        void this.#paint();
      }, seconds * 1000);
  }

  async #load() {
    // Loads overlap; only the one begun last is kept.
    const asked = ++this.#loads;
    const [persona, workgroups, talks, inbox] = await Promise.all([
      api.settings.getPersona({}),
      api.workgroups.listWorkgroups({}),
      api.console.listConversations({ workgroup: ALL, limit: 12, also: this.#talk ?? 0n }),
      api.console.getInbox({}),
    ]);
    const tz = persona.timezone || "Asia/Tokyo";
    const day = dayIn(Math.floor(Date.now() / 1000), tz);
    const [events, tasks, memos] = await Promise.all([
      api.calendar.listEvents({ from: day, to: addDays(day, 1), workgroup: ALL }),
      api.records.listRecords({ kind: "tasks", workgroup: ALL }),
      api.records.listRecords({ kind: "memos", workgroup: ALL }),
    ]);
    if (this.#talk === undefined && !this.#fresh) this.#talk = talks.conversations[0]?.id;
    if (asked !== this.#loads) return;
    this.#data = {
      aide: persona.name || "秘書",
      tz,
      workgroup: workgroups.workgroups.find((w) => w.id === ALL)?.name ?? "すべて",
      talks: talks.conversations,
      messages: this.#data.messages,
      approvals: inbox.approvals.filter((a) => a.status === "pending"),
      events: events.events.filter((e) => !e.allDay || localDate(e.start) <= day),
      tasks: tasks.records,
      memos: memos.records.sort((a, b) => b.updated.localeCompare(a.updated)),
    };
    await this.#loadMessages();
  }

  async #loadMessages() {
    if (this.#talk === undefined) {
      this.#data.messages = [];
      return;
    }
    const talk = this.#talk;
    const got = await api.console.listMessages({ workgroup: ALL, conversation: talk });
    // The owner may have opened another conversation meanwhile.
    if (this.#talk !== talk) return;
    this.#data.messages = got.messages;
    // What the glasses open on is read, as the console marks a conversation it opens.
    const last = got.messages.at(-1);
    if (last && last.id > got.readTo) await api.console.readConversation({ conversation: talk, message: last.id }).catch(() => undefined);
  }

  /** Keep up with what the secretary says and what changes, until the page goes. */
  async #follow() {
    for (;;) {
      try {
        const events = api.console.subscribeEvents({ workgroup: ALL }, { signal: this.#stop.signal });
        // What changed while no stream was open is read once it is.
        await this.#load()
          .then(() => this.#paint())
          .catch((error) => this.#fail(error));
        for await (const e of events) {
          // A refresh that fails is told; only the stream breaking makes it be followed anew.
          await this.#take(e.event).catch((error) => this.#fail(error));
        }
      } catch {
        if (this.#stop.signal.aborted) return;
      }
      await new Promise((r) => setTimeout(r, 2_000));
    }
  }

  /** What one change told by the console means for the glasses. */
  async #take(event: ConsoleEvent["event"]) {
    if (event.case === "message") {
      const m = event.value;
      // With no conversation yet, the first one begun is the one to show, unless the owner asked for a new one.
      if (this.#talk === undefined && !this.#fresh && m.conversation) this.#talk = m.conversation;
      if (m.who === Speaker.AIDE && m.conversation === this.#talk) this.#think(false, 0);
      if (m.conversation === this.#talk) await this.#loadMessages();
    } else if (event.case === "working") {
      // Only what was said from here is waited on; the console tells how long it lasts unless told again.
      if (this.#awaiting) this.#think(event.value.text !== "", event.value.seconds);
    } else {
      await this.#load();
    }
    await this.#paint();
  }


  stop() {
    this.#stop.abort();
    for (const timer of this.#timers) clearInterval(timer);
    this.#timers = [];
    clearTimeout(this.#thinkingUntil);
    if (this.#listening) void this.#close();
  }

  #top(): Top {
    const now = Math.floor(Date.now() / 1000);
    const day = dayIn(now, this.#data.tz);
    const [, m, d] = day.split("-").map(Number);
    return {
      clock: `${clock(now, this.#data.tz)}  ${m}/${d} ${WEEKDAYS[weekdayIndex(day)]}`,
      workgroup: this.#data.workgroup,
      confirms: this.#data.approvals.length,
      tasks: this.#ownTasks().length,
    };
  }

  #openTasks(): Rec[] {
    return this.#data.tasks.filter((t) => !isOver(t));
  }

  #ownTasks(): Rec[] {
    return this.#openTasks().filter((t) => [OWNER, ""].includes(valueOf(t, "Assignee")));
  }

  /** Tasks in the order shown: the owner's, then the secretary's, then what was done today. */
  #taskList(): Rec[] {
    const day = dayIn(Math.floor(Date.now() / 1000), this.#data.tz);
    const due = (t: Rec) => valueOf(t, "Due") || "9999";
    const mine = this.#ownTasks().sort((a, b) => due(a).localeCompare(due(b)));
    const others = this.#openTasks()
      .filter((t) => !mine.includes(t))
      .sort((a, b) => due(a).localeCompare(due(b)));
    const done = this.#data.tasks.filter((t) => isOver(t) && valueOf(t, "Finished").slice(0, 10) === day);
    return [...mine, ...others, ...done];
  }

  #taskRow(t: Rec): Row {
    const assignee = valueOf(t, "Assignee");
    const status = valueOf(t, "Status");
    const due = valueOf(t, "Due");
    const day = dayIn(Math.floor(Date.now() / 1000), this.#data.tz);
    const when = !due ? "" : due.slice(0, 10) === day ? hm(due) || "今日" : `${Number(due.slice(5, 7))}/${Number(due.slice(8, 10))}`;
    const who = assignee === SECRETARY ? this.#data.aide : assignee === WORKER ? "手代" : "";
    const note = [who, who ? statusWord(status) : when].filter(Boolean).join(" · ");
    const mark = isOver(t) ? "☑" : "□";
    return { text: `${pad(`${mark} ${clip(plainText(t.title), ROW_WIDTH - 190)}`, ROW_WIDTH - 170)}${note}`, level: isOver(t) ? 1 : 2 };
  }

  #nextEvent(): CalendarEvent | undefined {
    const now = Date.now();
    return this.#data.events.find((e) => !e.allDay && Date.parse(e.end) > now);
  }

  #untilText(e: CalendarEvent): string {
    const left = Math.ceil((Date.parse(e.start) - Date.now()) / 60_000);
    if (left <= 0) return "いま";
    return `あと ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
  }

  #eventTitle(e: CalendarEvent): string {
    return e.title || "予定あり";
  }

  /** The conversation, broken into the pages the glasses show, oldest first, and where the newest exchange begins. */
  #talkPages(): { all: Line[][]; newest: number } {
    const exchanges: Line[][] = [];
    for (const m of this.#data.messages) {
      const level: Level = m.who === Speaker.OWNER ? 2 : 4;
      const lines = wrap(plainText(m.text), TALK_WIDTH).map((text) => ({ text, level }));
      if (m.who === Speaker.OWNER || exchanges.length === 0) exchanges.push(lines);
      else exchanges[exchanges.length - 1].push({ text: "", level: 1 }, ...lines);
    }
    if (exchanges.length === 0) exchanges.push([]);
    const last = exchanges[exchanges.length - 1];
    const next = this.#nextEvent();
    if (next) last.unshift({ text: clip(`次 ${hm(next.start)} ${this.#eventTitle(next)}（${this.#untilText(next)}）`, TALK_WIDTH), level: 2 });
    for (const a of this.#shownApprovals()) {
      last.push({ text: "", level: 1 }, ...wrap(approvalText(a), TALK_WIDTH).map((text) => ({ text, level: 3 as Level })));
    }
    if (this.#thinking) last.push({ text: dots(this.#step), level: 4 });
    if (this.#notice) last.push({ text: "", level: 1 }, ...wrap(this.#notice, TALK_WIDTH).map((text) => ({ text, level: 2 as Level })));
    const before = exchanges.slice(0, -1).flatMap((lines) => pages(lines, TALK_LINES));
    return { all: [...before, ...pages(last, TALK_LINES)], newest: before.length };
  }

  /** Approvals asked in the conversation on screen. */
  #shownApprovals(): Approval[] {
    return this.#data.approvals.filter((a) => a.conversation !== 0n && a.conversation === this.#talk);
  }

  #page(): Page {
    const s = this.#screen;
    const top = this.#top();
    const art = this.#draw;
    switch (s.name) {
      case "talk": {
        const { all, newest } = this.#talkPages();
        const at = Math.min(s.page ?? newest, all.length - 1);
        return talkScreen(top, { lines: all[at], at, of: all.length }, art);
      }
      case "menu":
        return menuScreen(top, this.#menuItems(), s.selected, this.#card(MENU[s.selected].screen), art);
      case "talks": {
        const rows: Row[] = [{ text: "新しい話", level: 2 }, ...this.#data.talks.map((c) => this.#talkRow(c))];
        return listScreen(top, { icon: "talk", title: "会話", note: `${this.#data.talks.length} 件` }, rows, s.selected, [], art);
      }
      case "today": {
        const day = dayIn(Math.floor(Date.now() / 1000), this.#data.tz);
        const [, m, d] = day.split("-").map(Number);
        const next = this.#nextEvent();
        const rows = this.#data.events.map((e) => ({
          text: `${e.allDay ? "終日" : hm(e.start)}  ${clip(this.#eventTitle(e), ROW_WIDTH - 180)}${e === next ? `   ${this.#untilText(e)}` : ""}`,
          level: (next && this.#data.events.indexOf(e) < this.#data.events.indexOf(next) ? 1 : 2) as Level,
        }));
        return listScreen(top, { icon: "calendar", title: "今日の予定", note: `${m}/${d} ${WEEKDAYS[weekdayIndex(day)]} · ${rows.length} 件` }, rows, s.selected, [], art);
      }
      case "tasks": {
        const own = this.#ownTasks().length;
        const aide = this.#openTasks().length - own;
        return listScreen(top, { icon: "tasks", title: "タスク", note: `あなた ${own} · ${this.#data.aide} ${aide}` }, this.#taskList().map((t) => this.#taskRow(t)), s.selected, [], art);
      }
      case "task": {
        const t = this.#data.tasks.find((r) => r.id === s.id);
        if (!t) return this.#back();
        const due = valueOf(t, "Due");
        const note = [due ? `${Number(due.slice(5, 7))}/${Number(due.slice(8, 10))} ${hm(due)}`.trim() : "", valueOf(t, "Assignee") === SECRETARY ? `${this.#data.aide}の分` : "あなたの分"]
          .filter(Boolean)
          .join(" · ");
        return heldScreen(top, { icon: "tasks", title: clip(plainText(t.title), 300), note, body: [], at: 0, of: 1 }, this.#taskChoices(t), s.selected, art);
      }
      case "confirms": {
        const rows = this.#data.approvals.map((a) => ({
          text: `${pad(`□ ${clip(a.deed || a.tool, ROW_WIDTH - 110)}`, ROW_WIDTH - 90)}${clock(a.createdAt, this.#data.tz)}`,
          level: 2 as Level,
        }));
        return listScreen(top, { icon: "confirm", title: "確認", note: `待っている ${rows.length}` }, rows, s.selected, [], art);
      }
      case "confirm": {
        const a = this.#data.approvals.find((x) => x.id === s.id);
        if (!a) return this.#back();
        const body = this.#confirmPages(a, this.#confirmChoices(a, s.replies.length).length);
        const page = Math.min(s.page, body.length - 1);
        return heldScreen(
          top,
          { icon: "confirm", title: clip(a.deed || a.tool, 300), note: [a.place, clock(a.createdAt, this.#data.tz)].filter(Boolean).join(" · "), body: body[page], at: page, of: body.length },
          this.#confirmChoices(a, s.replies.length),
          s.selected,
          art,
        );
      }
      case "memos": {
        const memos = this.#data.memos;
        const rows = memos.map((r) => ({ text: `${pad(clip(plainText(r.title), ROW_WIDTH - 180), ROW_WIDTH - 170)}${shortDay(r.updated, this.#data.tz)}`, level: 2 as Level }));
        const chosen = memos[s.selected];
        const below = chosen ? wrap(excerptOf(chosen.notes, 80), ROW_WIDTH).slice(0, 2).map((text) => ({ text, level: 3 as Level })) : [];
        return listScreen(top, { icon: "memos", title: "メモ", note: `${memos.length} 件` }, rows, s.selected, below, art);
      }
      case "memo": {
        const r = this.#data.memos.find((x) => x.id === s.id);
        if (!r) return this.#back();
        const all = memoPages(r);
        const at = Math.min(s.page, all.length - 1);
        return talkScreen(top, { lines: all[at], at, of: all.length }, art);
      }
      case "listen": {
        const { all, newest } = this.#talkPages();
        const shown = all[newest] ?? [];
        return listenScreen(top, shown, s.about, `${s.heard}${dots(this.#step)}`, art);
      }
    }
  }

  /** Where a screen whose item is gone goes instead. */
  #back(): Page {
    this.#screen = { name: "talk", page: undefined };
    return this.#page();
  }

  #menuItems(): MenuItem[] {
    return MENU.map((m) => ({
      icon: m.icon,
      label: m.label,
      count: m.screen === "tasks" ? this.#ownTasks().length : m.screen === "confirms" ? this.#data.approvals.length : undefined,
    }));
  }

  #talkRow(c: Conversation): Row {
    const unread = c.unread ? `  ${c.unread}` : "";
    return { text: `${clip(c.title || "（題なし）", ROW_WIDTH - 60)}${unread}`, level: c.id === this.#talk ? 3 : 2 };
  }

  /** What the menu shows of an item, beside the list. */
  #card(screen: Screen["name"]): Card {
    const cut = (text: string) => clip(text, CARD_WIDTH);
    switch (screen) {
      case "talks": {
        const lines = this.#data.talks.slice(0, CARD_LINES).map((c) => ({ text: cut(c.title || "（題なし）"), level: (c.id === this.#talk ? 4 : 2) as Level }));
        return { icon: "talk", title: "会話", note: `${this.#data.talks.length} 件`, lines };
      }
      case "today": {
        const next = this.#nextEvent();
        const lines: Line[] = [];
        if (next) {
          lines.push({ text: cut(`${hm(next.start)}  ${this.#eventTitle(next)}`), level: 4 });
          lines.push({ text: cut([this.#untilText(next), next.meeting ? "Meet" : next.location].filter(Boolean).join(" · ")), level: 3 }, { text: "", level: 2 });
        }
        const after = next ? this.#data.events.slice(this.#data.events.indexOf(next) + 1) : [];
        lines.push(...after.map((e) => ({ text: cut(`${e.allDay ? "終日" : hm(e.start)}  ${this.#eventTitle(e)}`), level: 2 as Level })));
        if (!next) lines.push({ text: "これからの予定はない", level: 2 });
        return { icon: "calendar", title: "今日の予定", note: `${this.#data.events.length} 件`, lines };
      }
      case "tasks": {
        const list = this.#taskList().filter((t) => !isOver(t));
        const lines = list.map((t, i) => ({ text: cut(`□ ${plainText(t.title)}`), level: (i === 0 ? 4 : 2) as Level }));
        return { icon: "tasks", title: "タスク", note: `${this.#ownTasks().length} 件`, lines };
      }
      case "confirms": {
        const first = this.#data.approvals[0];
        const lines: Line[] = first
          ? [{ text: cut(first.deed || first.tool), level: 4 }, ...wrap(approvalText(first), CARD_WIDTH).map((text) => ({ text, level: 3 as Level }))]
          : [{ text: "待っているものはない", level: 2 }];
        return { icon: "confirm", title: "確認", note: `${this.#data.approvals.length} 件`, lines };
      }
      default: {
        const first = this.#data.memos[0];
        const lines: Line[] = first
          ? [
              { text: cut(plainText(first.title)), level: 4 },
              ...wrap(excerptOf(first.notes, 60), CARD_WIDTH).slice(0, 2).map((text) => ({ text, level: 3 as Level })),
              { text: "", level: 2 },
              ...this.#data.memos.slice(1).map((m) => ({ text: cut(plainText(m.title)), level: 2 as Level })),
            ]
          : [{ text: "メモはない", level: 2 }];
        return { icon: "memos", title: "メモ", note: `${this.#data.memos.length} 件`, lines };
      }
    }
  }

  /** What can be done with a task; one a worker runs is not handed to anyone else from here. */
  #taskActions(t: Rec): { label: string; act: "finish" | "hand" | "talk" }[] {
    const who = valueOf(t, "Assignee");
    const hand = who === WORKER ? [] : [{ label: who === SECRETARY ? "自分でやる" : `${this.#data.aide}に任せる`, act: "hand" as const }];
    return [{ label: isOver(t) ? "まだに戻す" : "済にする", act: "finish" }, ...hand, { label: "これについて話す", act: "talk" }];
  }

  #taskChoices(t: Rec): string[] {
    return this.#taskActions(t).map((a) => a.label);
  }

  #confirmPages(a: Approval, choices: number): string[][] {
    const lines = wrap(approvalText(a), HELD_WIDTH);
    const room = heldLines(choices);
    const all: string[][] = [];
    for (let i = 0; i < lines.length; i += room) all.push(lines.slice(i, i + room));
    return all.length ? all : [[]];
  }

  /** The answers an approval takes: its question's choices, one question at a time, or yes and no. */
  #confirmChoices(a: Approval, answered: number): string[] {
    const q = a.questions[answered];
    if (q) return q.choices.map((c) => c.label);
    return a.once ? ["許す", "やめる"] : ["許す", "やめる", "今後は聞かない"];
  }

  async #paint() {
    try {
      await this.#painter.show(this.#page());
    } catch (e) {
      this.onState(why(e));
    }
  }

  #count(): number {
    const s = this.#screen;
    switch (s.name) {
      case "menu":
        return MENU.length;
      case "talks":
        return this.#data.talks.length + 1;
      case "today":
        return this.#data.events.length;
      case "tasks":
        return this.#taskList().length;
      case "task": {
        const t = this.#data.tasks.find((r) => r.id === s.id);
        return t ? this.#taskChoices(t).length : 0;
      }
      case "confirms":
        return this.#data.approvals.length;
      case "confirm": {
        const a = this.#data.approvals.find((x) => x.id === s.id);
        return a ? this.#confirmChoices(a, s.replies.length).length : 0;
      }
      case "memos":
        return this.#data.memos.length;
      default:
        return 0;
    }
  }

  async #on(input: Input) {
    if (input.kind === "sound") return this.#hear(input.pcm);
    if (this.#sending && input.kind !== "cleared" && input.kind !== "gone") return;
    if (input.kind === "cleared") {
      this.#painter.forget();
      return this.#paint();
    }
    if (input.kind === "stayed") return this.#paint();
    if (input.kind === "gone") return this.stop();
    if (input.kind !== "up" && input.kind !== "down") this.#notice = "";
    const s = this.#screen;
    if (s.name === "listen") {
      if (input.kind === "tap") return this.#finish();
      if (input.kind === "double") return this.#cancel();
      return;
    }
    if (input.kind === "long") {
      this.#screen = { name: "menu", selected: 0 };
      return this.#paint();
    }
    if (input.kind === "double") return this.#goBack();
    if (input.kind === "up" || input.kind === "down") return this.#move(input.kind === "up" ? -1 : 1);
    return this.#tap();
  }

  async #goBack() {
    const s = this.#screen;
    switch (s.name) {
      case "talk":
        if (s.page !== undefined) this.#screen = { name: "talk", page: undefined };
        else return this.#glasses.leave();
        break;
      case "task":
        this.#screen = { name: "tasks", selected: Math.max(0, this.#taskList().findIndex((t) => t.id === s.id)) };
        break;
      case "confirm":
        this.#screen = { name: "confirms", selected: Math.max(0, this.#data.approvals.findIndex((a) => a.id === s.id)) };
        break;
      case "memo":
        this.#screen = { name: "memos", selected: Math.max(0, this.#data.memos.findIndex((m) => m.id === s.id)) };
        break;
      default:
        this.#screen = { name: "talk", page: undefined };
    }
    return this.#paint();
  }

  async #move(by: -1 | 1) {
    const s = this.#screen;
    if (s.name === "talk") {
      const { all, newest } = this.#talkPages();
      const next = Math.min(Math.max(0, (s.page ?? newest) + by), all.length - 1);
      this.#screen = { name: "talk", page: next === newest ? undefined : next };
    } else if (s.name === "memo") {
      const r = this.#data.memos.find((x) => x.id === s.id);
      const last = r ? memoPages(r).length - 1 : 0;
      this.#screen = { ...s, page: Math.min(Math.max(0, s.page + by), last) };
    } else if (s.name === "confirm") {
      const a = this.#data.approvals.find((x) => x.id === s.id);
      const pagesOf = a ? this.#confirmPages(a, this.#confirmChoices(a, s.replies.length).length).length : 1;
      const last = this.#count() - 1;
      // The whole text comes before the answers: past the ends, the text turns its pages.
      // Turning a page of the text puts the choice back on the first answer, so reading on never lands on 今後は聞かない.
      if (by < 0 && s.selected === 0 && s.page > 0) this.#screen = { ...s, page: s.page - 1, selected: 0 };
      else if (by > 0 && s.selected === last && s.page < pagesOf - 1) this.#screen = { ...s, page: s.page + 1, selected: 0 };
      else this.#screen = { ...s, selected: Math.min(Math.max(0, s.selected + by), last) };
    } else if ("selected" in s) {
      this.#screen = { ...s, selected: Math.min(Math.max(0, s.selected + by), Math.max(0, this.#count() - 1)) };
    }
    return this.#paint();
  }

  async #tap() {
    const s = this.#screen;
    switch (s.name) {
      case "talk":
        return this.#listen(undefined);
      case "menu": {
        const to = MENU[s.selected].screen;
        const next = this.#nextEvent();
        // Today opens on what comes next; the rest open on their first item.
        if (to === "today") this.#screen = { name: to, selected: next ? this.#data.events.indexOf(next) : 0 };
        else if (to === "talks" || to === "tasks" || to === "confirms" || to === "memos") this.#screen = { name: to, selected: 0 };
        return this.#paint();
      }
      case "talks": {
        // What was waited on in the conversation left is not waited on here.
        this.#think(false, 0);
        this.#notice = "";
        if (s.selected === 0) {
          this.#fresh = true;
          this.#talk = undefined;
        } else {
          this.#fresh = false;
          this.#talk = this.#data.talks[s.selected - 1]?.id;
        }
        await this.#loadMessages();
        this.#screen = { name: "talk", page: undefined };
        return this.#paint();
      }
      case "today": {
        const e = this.#data.events[s.selected];
        if (!e) return;
        return this.#listen(`予定：${e.allDay ? "終日" : hm(e.start)} ${this.#eventTitle(e)}`);
      }
      case "tasks": {
        const t = this.#taskList()[s.selected];
        if (!t) return;
        this.#screen = { name: "task", id: t.id, selected: 0 };
        return this.#paint();
      }
      case "task": {
        const t = this.#data.tasks.find((r) => r.id === s.id);
        if (!t) return;
        const act = this.#taskActions(t)[s.selected]?.act;
        if (!act) return;
        if (act === "talk") return this.#listen(`タスク：${plainText(t.title)}`);
        const [name, value] =
          act === "finish" ? ["Status", isOver(t) ? "pending" : "done"] : ["Assignee", valueOf(t, "Assignee") === SECRETARY ? OWNER : SECRETARY];
        await api.records.saveRecord({ kind: "tasks", id: t.id, fields: [{ name, value }] });
        await this.#load();
        this.#screen = { name: "tasks", selected: Math.max(0, this.#taskList().findIndex((r) => r.id === t.id)) };
        return this.#paint();
      }
      case "confirms": {
        const a = this.#data.approvals[s.selected];
        if (!a) return;
        this.#screen = { name: "confirm", id: a.id, selected: 0, page: 0, replies: [] };
        return this.#paint();
      }
      case "confirm":
        return this.#answer(s);
      case "memos": {
        const r = this.#data.memos[s.selected];
        if (!r) return;
        this.#screen = { name: "memo", id: r.id, page: 0 };
        return this.#paint();
      }
      case "memo": {
        const r = this.#data.memos.find((x) => x.id === s.id);
        if (r) return this.#listen(`メモ：${plainText(r.title)}`);
      }
    }
  }

  async #answer(s: Extract<Screen, { name: "confirm" }>) {
    const a = this.#data.approvals.find((x) => x.id === s.id);
    if (!a) return;
    const q = a.questions[s.replies.length];
    if (q) {
      const replies = [...s.replies, q.choices[s.selected]?.label ?? ""];
      if (replies.length < a.questions.length) {
        this.#screen = { ...s, selected: 0, replies };
        return this.#paint();
      }
      await api.console.answerApproval({ id: a.id, answer: ApprovalAnswer.ALLOW, replies });
    } else {
      const answer = [ApprovalAnswer.ALLOW, ApprovalAnswer.DENY, ApprovalAnswer.ALWAYS][s.selected];
      await api.console.answerApproval({ id: a.id, answer });
    }
    await this.#load();
    this.#screen = this.#data.approvals.length ? { name: "confirms", selected: 0 } : { name: "talk", page: undefined };
    return this.#paint();
  }

  async #listen(about: string | undefined) {
    const back = this.#screen;
    this.#screen = { name: "listen", about, heard: "", back };
    const listening = new Listening();
    this.#listening = listening;
    this.#hearing = true;
    await this.#paint();
    const open = await this.#glasses.microphone(true);
    // Sent or put away while the microphone was opening: it is not wanted now.
    if (this.#listening !== listening) {
      if (open) await this.#glasses.microphone(false);
      return;
    }
    if (open) {
      // The samples count the time; with none coming, the clock gives up instead.
      this.#silentUntil = setTimeout(() => {
        if (this.#listening === listening && listening.ms() === 0) void this.#cancel();
      }, NOTHING_MS);
    }
    if (!open) {
      // It may open after the page gave up waiting.
      await this.#glasses.microphone(false);
      this.#hearing = false;
      this.#listening = undefined;
      this.#screen = back;
      this.#fail(new Error("マイクを開けなかった"));
    }
  }

  async #hear(pcm: Uint8Array) {
    const listening = this.#listening;
    const s = this.#screen;
    if (!listening || s.name !== "listen") return;
    const heard = listening.take(pcm);
    if (heard === "nothing") return this.#cancel();
    if (heard === "done") return this.#finish();
    // One hearing of what was said so far at a time, so they neither pile up nor land out of order.
    if (!this.#interim && listening.due()) {
      this.#interim = true;
      const so = await api.console.transcribe({ pcm: listening.sound(), rate: RATE, preview: true }).catch(() => undefined);
      this.#interim = false;
      if (so && this.#listening === listening && this.#screen.name === "listen") {
        this.#screen = { ...this.#screen, heard: so.text };
        await this.#paint();
      }
    }
  }

  async #close(): Promise<Listening | undefined> {
    clearTimeout(this.#silentUntil);
    const listening = this.#listening;
    this.#listening = undefined;
    await this.#glasses.microphone(false);
    return listening;
  }

  async #cancel() {
    await this.#close();
    this.#hearing = false;
    const s = this.#screen;
    this.#screen = s.name === "listen" ? s.back : s;
    return this.#paint();
  }

  async #finish() {
    const s = this.#screen;
    if (s.name !== "listen" || this.#sending) return;
    // From here what was said goes; nothing the owner does stops it halfway.
    this.#sending = true;
    const listening = await this.#close();
    try {
      const text = listening?.spoke() ? (await api.console.transcribe({ pcm: listening.sound(), rate: RATE })).text.trim() : "";
      this.#hearing = false;
      if (!text) {
        this.#screen = s.back;
        return this.#paint();
      }
      this.#screen = { name: "talk", page: undefined };
      await this.#say(text, s.about, s.back);
    } catch (e) {
      this.#hearing = false;
      this.#think(false, 0);
      this.#screen = { name: "talk", page: undefined };
      this.#fail(e);
    } finally {
      this.#sending = false;
    }
  }

  async #say(text: string, about: string | undefined, from: Screen) {
    const shown = this.#shownApprovals();
    // An approval that asks questions is answered by choosing, from the menu's 確認.
    const answer = !about && shown.length === 1 && shown[0].questions.length === 0 ? spokenAnswer(text) : undefined;
    // The words go to the secretary whether or not the answer could be given.
    if (answer !== undefined) await api.console.answerApproval({ id: shown[0].id, answer }).catch((e) => this.#fail(e));
    this.#awaiting = true;
    this.#think(true, THINKING_SECONDS);
    await this.#paint();
    const sent = await api.console.send({
      text,
      workgroup: ALL,
      view: from.name === "talk" ? VIEW : `${VIEW} · ${screenWord(from)}`,
      selection: about ?? "",
      conversation: this.#talk ?? 0n,
      newConversation: this.#talk === undefined,
      device: "g2",
    });
    this.#fresh = false;
    if (sent.conversation) this.#talk = sent.conversation;
    await this.#load();
    await this.#paint();
  }
}

/** The pages `lines` take in boxes of `room` lines, each marked with where it is when there are several. */
export function pages(lines: Line[], room: number): Line[][] {
  if (lines.length <= room) return [lines];
  const per = room - 1;
  const count = Math.ceil(lines.length / per);
  return Array.from({ length: count }, (_, i) => [...lines.slice(i * per, (i + 1) * per), { text: `(${i + 1}/${count})`, level: 1 as Level }]);
}

/** A memo's whole text, a page at a time. */
function memoPages(r: Rec): Line[][] {
  return pages([{ text: plainText(r.title), level: 4 }, ...wrap(plainText(r.notes), TALK_WIDTH).map((text) => ({ text, level: 3 as Level }))], TALK_LINES);
}

function statusWord(status: string): string {
  return { in_progress: "進めている", waiting: "返事待ち", pending: "これから" }[status] ?? "";
}

function screenWord(s: Screen): string {
  return { talk: "会話", menu: "メニュー", talks: "会話", today: "今日の予定", tasks: "タスク", task: "タスク", confirms: "確認", confirm: "確認", memos: "メモ", memo: "メモ", listen: "会話" }[s.name];
}

function shortDay(at: string, tz: string): string {
  if (!at) return "";
  const day = at.slice(0, 10);
  const today = dayIn(Math.floor(Date.now() / 1000), tz);
  if (day === today) return "今日";
  if (day === addDays(today, -1)) return "昨日";
  return `${Number(day.slice(5, 7))}/${Number(day.slice(8, 10))}`;
}
