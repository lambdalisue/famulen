// Threads: the unit the owner talks in. Approvals and results come to the thread that asked for them.
const THREADS = [
  { id: "t1", t: "松本さんに返事を書いて。論点は三つに絞ってあるやつ。", wg: "work", at: "10:41", group: "今日", asks: ["mail"], unread: 1, turns: 4, ctx: 18 },
  { id: "t2", t: "引っ越し業者の見積もりを比べて、1 社に絞って", wg: "home", at: "10:49", group: "今日", asks: ["question"], unread: 1, job: "j1", turns: 6, ctx: 24 },
  { id: "t3", t: "椅子の座面が破れた。部品だけ替えられる？", wg: "home", at: "10:54", group: "今日", from: "slack", unread: 0, turns: 2, ctx: 9 },
  { id: "t4", t: "四半期報告の数字を、基幹と突き合わせて", wg: "work", at: "10:20", group: "今日", unread: 0, job: "j3", busy: "結果を読んでいる", turns: 11, ctx: 61 },
  { id: "t5", t: "ふるさと納税の控除上限を試算して", wg: "home", at: "8:40", group: "今日", unread: 2, job: "j7", result: 1, turns: 3, ctx: 14 },
  { id: "t6", t: "経費精算の CSV を新しい書式に合わせて", wg: "work", at: "8:52", group: "今日", asks: ["start"], unread: 1, job: "j2", turns: 5, ctx: 22 },
  { id: "t7", t: "朝の支度：今日の予定と、夜のうちに届いたもの", wg: "", at: "7:30", group: "今日", from: "routine", unread: 0, turns: 1, ctx: 7 },
  { id: "t8", t: "北星ネットの請求の一覧を読めるようにして", wg: "home", at: "昨日", group: "以前", asks: ["definition"], unread: 0, turns: 7, ctx: 33 },
  { id: "t9", t: "共有カレンダーに忘年会の候補日を仮押さえして", wg: "work", at: "昨日", group: "以前", asks: ["cut"], unread: 0, turns: 2, ctx: 8 },
  { id: "t10", t: "古い写真を年ごとのアルバムに振り分けて", wg: "home", at: "昨日", group: "以前", unread: 0, job: "j8", turns: 4, ctx: 16 },
];
const ORIGIN = { slack: ["brand-slack", "Slack から"], routine: ["repeat", "定期"], watch: ["eye", "見張り"] };
const ASK_WORD = { mail: "承認待ち", question: "質問", start: "承認待ち", definition: "承認待ち", cut: "途中で切れた" };
const conv = { view: "thread", id: "t3" };
const drafts = {};
/** In the order they came: what the owner said, and the result that reached the thread. */
const said = {};
const RESULT = Symbol("result");
let arrived = null;
const thread = (id) => THREADS.find((t) => t.id === id);
const asksOf = (t) => (t.asks ?? []).filter((k) => !answered[k]);
const convKind = () => root.dataset.conv ?? "drill";
const destKind = () => root.dataset.dest ?? "seg";
const destKey = () => (conv.view === "thread" ? conv.id : "new");

const mine = (text, w, at, from = "") => `<div class="me"><div class="say">${text}</div><div class="meta">${wgMark(w)}${from ? `<span class="row" style="gap:4px">${i(ORIGIN[from][0])}${ORIGIN[from][1]}</span>` : ""}<span>${at}</span></div></div>`;
const hers = (at, body, note = "") => `<div class="her">${face(30)}<div class="body"><div class="who"><b>${AIDE}</b><span>${at}</span>${note}<span class="gap"></span>${btn("", { v: "icon ghost sm quote-it", icon: "arrow-back-up", act: "quote", attrs: 'aria-label="この発言を引いて返す" title="この発言を引いて返す"' })}</div>${body}</div></div>`;
const prose = (...ps) => `<div class="prose">${ps.map((p) => `<p>${p}</p>`).join("")}</div>`;
const fresh = '<div class="fresh" role="separator"><span>ここから新しい</span></div>';
const resultCard = (j, text) => `<article class="result card"><header>${i(j.st === "failed" ? "alert-triangle" : "circle-check")}<b>委ねた仕事の結果が届いた</b><span class="gap"></span><span class="xs muted">${j.to}</span></header><div class="in"><b>${j.t}</b><p class="sm">${text}</p><div class="row wrap">${btn("仕事を開く", { v: "sm", icon: "checklist", act: "openjob", attrs: `data-job="${j.id}"` })}</div></div></article>`;
const job = (id) => JOBS.find((j) => j.id === id);
const TORN = '<button type="button" class="attach" aria-label="添えた画像「座面の破れ」を大きく見る" data-act="image"><svg viewBox="0 0 200 120" aria-hidden="true"><rect width="200" height="120" fill="var(--pressed)"/><path d="M0 34Q100 8 200 34V96Q100 120 0 96Z" fill="light-dark(#66718C,#4A5368)"/><path d="M0 84Q100 108 200 84" fill="none" stroke="light-dark(#8D97AE,#68728A)" stroke-width="2" stroke-dasharray="5 4"/><path d="M78 46l16 10 8-4 14 14 12 2-14 5-10-2-14-8-6-7z" fill="light-dark(#E9DFC9,#C9BC9F)"/></svg></button>';
const LINK = '<a class="card linkcard" href="#chat" data-act="todo"><div><div class="sm" style="font-weight:600">設計レビューの論点</div><div class="xs muted">集計の単位、締めの時刻、過去分の扱いについて</div><div class="xs muted row" style="margin-top:4px">docs.example.com</div></div><svg viewBox="0 0 88 72" preserveAspectRatio="xMidYMid slice" aria-hidden="true"><rect width="88" height="72" fill="var(--sunken)"/><path d="M18 16h52v48H18z" fill="var(--surface)" stroke="var(--line)"/><path d="M26 27h26M26 36h36M26 44h36M26 52h22" stroke="var(--control)" stroke-width="3"/></svg></a>';
const thinking = (t) => `<div class="thinking" role="status"><span class="pulse"><i></i><i></i><i></i></span>${t}</div>`;
/** What is said in each thread, in order. */
const SAID = {
  t1: (t) => [mine(t.t, t.wg, "10:38"), hers("10:41", `${prose("下書きしました。<b>集計の単位</b>、<b>締めの時刻</b>、<b>過去分の扱い</b>の三つです。資料はここに置いてあります。")}${LINK}<div class="row wrap">${btn("タスクを開く", { v: "sm", icon: "checklist", act: "go", attrs: 'data-go="tasks"' })}</div>${t.marked ? fresh : ""}${request("mail")}`)],
  t2: (t) => [mine(t.t, t.wg, "10:02"), hers("10:03", prose("手代に委ねました。3 社の見積もりが揃ったら比べます。")), t.marked ? fresh : "", hers("10:49", `${prose("比べる前に、一つ確かめさせてください。")}${request("question")}`, badge("手代から", "line", "robot"))],
  t3: (t) => [`<div class="me">${TORN}<div class="say">${t.t}</div><div class="meta">${wgMark(t.wg)}<span class="row" style="gap:4px">${i("brand-slack")}Slack から</span><span>10:52</span></div></div>`, hers("10:54", `${prose("替えられます。座面だけなら <code>¥14,300</code> で、保証はあと 12 日残っています。保証で直せるかを先に尋ねますか。")}<div class="row wrap"><a class="chip" href="#things" data-open="things:t2">${i("layout-cards")}ワークチェア</a></div>`)],
  t4: (t) => [mine(t.t, t.wg, "9:38"), hers("9:40", prose("始めました。食い違いは、行と理由を一覧にして渡します。")), hers("10:20", prose("集計の取り出しと列の対応まで済みました。食い違いは今のところ 7 行です。")), t.busy ? thinking(t.busy) : ""],
  t5: (t) => [mine(t.t, t.wg, "8:24"), hers("8:25", prose("手代に委ねました。")), t.marked ? fresh : "", resultCard(job("j7"), "上限の目安は 112,000 円。医療費控除を入れると 104,000 円。"), hers("8:40", prose("医療費控除の明細がまだ揃っていないので、低いほうを目安にするのがよいと思います。"))],
  t6: (t) => [mine(t.t, t.wg, "8:50"), hers("8:52", `${prose("Adroit に委ねます。始めてよいか、確かめさせてください。")}${t.marked ? fresh : ""}${request("start")}`)],
  t7: () => [hers("7:30", '<div class="prose"><p>おはようございます。今日は予定が 4 件あります。午後は 14:30 から空いています。</p><ul><li>11:00 の設計レビューは、松本さんが論点の返事を待っています</li><li>写真編集ソフトの支払いが 12月3日にあります</li></ul></div>', `<span class="meta">${i("repeat")}朝の支度</span>`)],
  t8: (t) => [mine(t.t, t.wg, "昨日"), hers("昨日", `${prose("定義を直す案です。")}${request("definition")}`)],
  t9: (t) => [mine(t.t, t.wg, "昨日 23:08"), request("cut")],
  t10: (t) => [mine(t.t, t.wg, "昨日 22:50"), hers("昨日 22:51", prose("手代に委ねました。")), resultCard(job("j8"), "写真の置き場所に届かなかった。つなぎ先を直すと、また動かせる。")],
};
const lastOf = (t) => {
  const s = said[t.id]?.at(-1);
  if (s === RESULT) return ["Adroit", "食い違いは 7 行。うち 5 行は締めの時刻の違い。"];
  if (s) return ["あなた", s];
  return { t1: ["ハル", "下書きしました。集計の単位、締めの時刻、過去分の扱いの三つです。"], t2: ["手代", "荷物の量は、どれに近いですか"], t3: ["ハル", "替えられます。座面だけなら ¥14,300 で、保証はあと 12 日残っています。"], t4: ["ハル", "集計の取り出しと列の対応まで済みました。"], t5: ["手代", "上限の目安は 112,000 円。"], t6: ["ハル", "Adroit に委ねます。始めてよいか、確かめさせてください。"], t7: ["ハル", "おはようございます。今日は予定が 4 件あります。"], t8: ["ハル", "定義を直す案です。"], t9: ["ハル", "途中で止まった。終わったか分からない。"], t10: ["手代", "写真の置き場所に届かなかった。"] }[t.id] ?? ["あなた", t.t];
};
const body = (t) => `${SAID[t.id](t).join("")}${(said[t.id] ?? []).map((x) => x === RESULT ? `${t.marked ? fresh : ""}${resultCard(job("j3"), "食い違いは 7 行。うち 5 行は締めの時刻の違いで、残りは重複。")}` : `<div class="me"><div class="say" data-said="${t.id}"></div><div class="meta"><span>いま</span></div></div>`).join("")}`;

/* ---------- the list ---------- */
const shownThreads = () => THREADS.filter(inWg).filter((t) => ({ ask: asksOf(t).length, unread: t.unread, busy: t.busy, "": true })[conv.filter ?? ""]);
const threadBadges = (t) => `${asksOf(t).map((k) => badge(ASK_WORD[k], "warn", k === "question" ? "message-circle" : k === "cut" ? "alert-triangle" : "hand-stop")).join("")}${t.result && t.unread ? badge("結果が届いた", "ok", "check") : ""}${t.busy ? badge("動いている", "info", "refresh", "run") : ""}`;
const threadRow = (t) => {
  const on = conv.view === "thread" && conv.id === t.id;
  return `<li class="item th ${t.unread ? "unread" : ""} ${on && convKind() === "side" ? "on" : ""}"><div class="main"><div class="row"><button type="button" class="pick t cut grow" data-thread="${t.id}" ${on ? 'aria-current="true"' : ""} data-title="${t.id}"></button><span class="xs muted">${t.at}</span></div><div class="row"><span class="xs muted cut grow" data-last="${t.id}"></span>${t.unread ? `<span class="count new" aria-label="未読 ${t.unread}">${t.unread}</span>` : ""}</div><div class="sub">${threadBadges(t)}${wgMark(t.wg)}${t.from ? `<span class="row" style="gap:4px">${i(ORIGIN[t.from][0])}${ORIGIN[t.from][1]}</span>` : ""}</div></div></li>`;
};
const threadList = () => {
  const all = THREADS.filter(inWg);
  const n = { ask: all.filter((t) => asksOf(t).length).length, unread: all.filter((t) => t.unread).length, busy: all.filter((t) => t.busy).length };
  const groups = [["あなたの答え待ち", (t) => asksOf(t).length], ["新しく届いた", (t) => !asksOf(t).length && t.unread], ["今日", (t) => !asksOf(t).length && !t.unread && t.group === "今日"], ["以前", (t) => !asksOf(t).length && !t.unread && t.group === "以前"]];
  return `<div class="tlist" tabindex="0" aria-label="話の一覧"><div class="sumrow" role="group" aria-label="話を絞る">${[["", "すべて", all.length], ["ask", "答え待ち", n.ask], ["unread", "未読", n.unread], ["busy", "動いている", n.busy]].map(([k, l, c]) => `<button type="button" class="chip" data-act="tfilter" data-v="${k}" aria-pressed="${(conv.filter ?? "") === k}">${l} ${c}</button>`).join("")}</div>
  ${groups.map(([g, f]) => [g, shownThreads().filter(f)]).filter(([, ts]) => ts.length).map(([g, ts]) => `<section class="stack" style="gap:6px"><div class="group"><h2>${g}</h2><span class="muted">${ts.length}</span></div><ul class="list">${ts.map(threadRow).join("")}</ul></section>`).join("") || empty("当てはまる話はない")}</div>`;
};

/* ---------- the head, the body, where it goes ---------- */
const unreadElsewhere = () => THREADS.filter((t) => inWg(t) && t.id !== (conv.view === "thread" ? conv.id : "") && (t.unread || asksOf(t).length)).length;
const convHead = (kind) => {
  const list = kind !== "side" && kind !== "stream" && conv.view === "list";
  const t = conv.view === "thread" ? thread(conv.id) : null;
  const n = unreadElsewhere();
  return `<header class="chead">${list || kind === "stream" ? `${face(28)}<b class="grow">${AIDE}との話</b>` : `<button type="button" class="btn ghost sm back-list ${kind === "side" ? "m-only" : ""}" data-act="tlist" aria-label="話の一覧へ${n ? `（ほかに ${n} 件が待っている）` : ""}">${i("chevron-left")}${n ? `<span class="count warn">${n}</span>` : ""}</button><div class="grow"><b class="cut" ${t ? `data-title="${t.id}"` : ""}>${t ? "" : "新しい話"}</b>${t ? `<details class="about"><summary>${wgMark(t.wg)}${t.from ? `<span>${ORIGIN[t.from][1]}</span>` : ""}<span>${t.at}</span>${i("chevron-right")}</summary><dl class="kv"><dt>やりとり</dt><dd>${t.turns} 回</dd><dt>いまの文脈</dt><dd><span class="meter" style="display:block;max-width:140px;margin-top:6px"><i style="width:${t.ctx}%"></i></span><span class="xs muted">${t.ctx}%。増えると自動で縮む。発言は消えない</span></dd>${t.job ? `<dt>委ねた仕事</dt><dd><button type="button" class="chip" data-act="openjob" data-job="${t.job}">${i("checklist")}${job(t.job).t}</button></dd>` : ""}</dl></details>` : '<span class="xs muted">これまでの話は引き継がない</span>'}</div>`}${btn("新しい話", { v: "sm", icon: "message-plus", act: "tnew" })}${ib("chevron-down", "会話を畳む", "fold", "ghost sm m-only")}</header>`;
};
const newBody = () => `<div class="newtalk">${face(48)}<h2>新しい話を始める</h2><p class="sm muted">前の話は引き継がない。続きを話すなら、下から選ぶ。</p><ul class="list">${THREADS.filter((t) => inWg(t) && !t.from).slice(0, 2).map(({ id }) => `<li class="item mid"><div class="main"><button type="button" class="pick t cut" data-thread="${id}" data-title="${id}"></button><div class="sub">続きを話す、${thread(id).at}</div></div>${i("chevron-right")}</li>`).join("")}</ul></div>`;
const talkBody = () => (conv.view === "thread" ? body(thread(conv.id)) : newBody());
const stream = () => THREADS.filter(inWg).slice().reverse().map((t) => {
  const on = conv.view === "thread" && conv.id === t.id;
  return `<section class="card tcard ${on ? "on" : ""}" aria-label="話"><div class="tc-head"><button type="button" class="pick t cut grow" data-thread="${t.id}" data-title="${t.id}"></button>${t.unread ? `<span class="count new">${t.unread}</span>` : ""}<span class="xs muted">${t.at}</span></div>${on ? `<div class="tc-body">${body(t)}</div>` : `<div class="tc-foot"><span class="xs muted cut" data-last="${t.id}"></span><div class="sub row wrap">${threadBadges(t)}</div></div>`}</section>`;
}).join("");
const convHtml = (kind) => kind === "stream"
  ? `${convHead(kind)}<div class="talk stream" tabindex="0" aria-label="会話">${stream()}${conv.view === "new" ? newBody() : ""}</div>`
  : kind === "side"
    ? `<div class="two ${conv.view === "list" ? "at-list" : ""}"><div class="side-list"><header class="chead"><b class="grow">話</b>${ib("chevron-down", "会話を畳む", "fold", "ghost sm m-only")}</header>${threadList()}</div><div class="side-talk">${convHead(kind)}<div class="talk" tabindex="0" aria-label="会話">${conv.view === "list" ? newBody() : talkBody()}</div></div></div>`
    : `${convHead(kind)}${conv.view === "list" ? threadList() : `<div class="talk" tabindex="0" aria-label="会話">${talkBody()}</div>`}`;
/** Where what is written goes, said at every moment. */
const destHtml = (kind) => {
  const t = conv.view === "thread" ? thread(conv.id) : null;
  return kind === "seg"
    ? `<div class="dest" role="group" aria-label="送り先"><div class="segs"><button type="button" class="seg" aria-pressed="${!!t}" data-act="tcontinue" ${THREADS.some(inWg) ? "" : "disabled"}>続き</button><button type="button" class="seg" aria-pressed="${!t}" data-act="tnew" data-carry="1">新しい話</button></div><span class="cut grow dest-name">${t ? `<span data-title="${t.id}"></span>` : "題は最初の一文から付く"}</span></div>`
    : `<div class="dest"><span class="xs muted">送り先</span><button type="button" class="chip dest-chip" data-act="tlist" aria-label="送り先を選ぶ">${i(t ? "message-circle" : "message-plus")}<span class="cut">${t ? `<span data-title="${t.id}"></span>` : "新しい話"}</span>${i("chevron-down")}</button>${t ? `<button type="button" class="btn ghost sm" data-act="tnew" data-carry="1">新しい話にする</button>` : ""}</div>`;
};
const arrivalHtml = () => (arrived && !(conv.view === "thread" && arrived === conv.id) ? `<div class="arrival" role="status">${i("bell")}<div class="grow"><b class="sm">委ねた仕事の結果が届いた</b><div class="xs cut" data-title="${arrived}"></div></div>${btn("開く", { v: "sm", act: "topen", attrs: `data-id="${arrived}"` })}${ib("x", "あとで", "tlater")}</div>` : "");

/** Titles and last words are what people said, so they are put in as text. */
function fillThreads(box = document) {
  $$("[data-title]", box).forEach((el) => { el.textContent = thread(el.dataset.title).t; });
  $$("[data-last]", box).forEach((el) => { const [w, x] = lastOf(thread(el.dataset.last)); el.textContent = `${w}：${x}`; });
  for (const t of THREADS) $$(`[data-said="${t.id}"]`, box).forEach((el, n) => { const words = said[t.id].filter((x) => x !== RESULT); el.textContent = words[n % words.length]; });
}
function goConv(view, id) {
  drafts[destKey()] = $("#say").value;
  // What is new stays marked for as long as the thread is open.
  if (view === "thread") {
    for (const t of THREADS) if (t.id !== id) t.marked = false;
    const t = thread(id);
    t.marked ||= t.unread > 0;
    t.unread = 0;
    conv.id = id;
  }
  conv.view = view;
  if (arrived === id) arrived = null;
  $("#say").value = drafts[destKey()] ?? "";
  const held = document.activeElement;
  const inConv = held?.closest?.("#conv, #arrival, #dest");
  draw(true, true);
  if (inConv && !document.contains(held)) ($(view === "list" ? `#conv [data-thread="${conv.id}"]` : "#conv .talk") ?? $("#conv .tlist"))?.focus({ preventScroll: true });
}
const THREAD_ACTS = {
  tlist: () => goConv("list"),
  tnew: (d) => { if (d.carry && conv.view === "thread") { const x = $("#say").value; goConv("new"); if (x && !$("#say").value) { $("#say").value = x; drafts[conv.id] = ""; } } else goConv("new"); $("#say").focus({ preventScroll: true }); },
  tcontinue: () => { const to = [thread(conv.id), ...THREADS].find(inWg); if (!to) return; const x = $("#say").value; goConv("thread", to.id); if (x && !$("#say").value) { $("#say").value = x; drafts.new = ""; } $("#say").focus({ preventScroll: true }); },
  topen: (d) => goConv("thread", d.id),
  tlater: () => { arrived = null; draw(true); },
  tfilter: (d) => { conv.filter = d.v; redraw(); },
  arrive: () => { const t = thread("t4"); delete t.busy; const open = conv.view === "thread" && conv.id === "t4" && (chatOpen || !narrow()); t.unread = open ? 0 : 1; t.marked = open; t.result = 1; t.at = "いま"; arrived = open ? null : "t4"; (said.t4 ??= []).push(RESULT); draw(true); toast("委ねた仕事の結果が届いた"); },
  quote: (d, t) => { const text = t.closest(".her").querySelector(".prose")?.textContent.trim().slice(0, 18) ?? "発言"; const q = `「${text}…」への返事`; if (!attached.includes(q)) attached.push(q); draw(true); $("#say").focus({ preventScroll: true }); },
};
function sendSaid(v) {
  if (conv.view !== "thread") {
    const id = `n${THREADS.length}`;
    THREADS.unshift({ id, t: v.length > 24 ? `${v.slice(0, 24)}…` : v, wg: wg === "all" ? "" : wg, at: "いま", group: "今日", unread: 0, turns: 1, ctx: 2 });
    SAID[id] = () => [];
    conv.view = "thread";
    conv.id = id;
  }
  (said[conv.id] ??= []).push(v);
  thread(conv.id).at = "いま";
}

/* ---------- side by side ---------- */
const frame = (cap, html) => `<figure class="tryframe" inert><div class="fr">${html}</div><figcaption class="xs muted">${cap}</figcaption></figure>`;
function threadsPage() {
  const was = { ...conv };
  const as = (view, id, f) => { conv.view = view; conv.id = id ?? was.id; const h = f(); Object.assign(conv, was); return h; };
  const pickConv = (v) => btn(convKind() === v ? "いま使っている" : "この案で使う", { v: convKind() === v ? "soft sm" : "sm", icon: convKind() === v ? "check" : "", attrs: `data-choice="conv:${v}"` });
  const pickDest = (v) => btn(destKind() === v ? "いま使っている" : "この案で使う", { v: destKind() === v ? "soft sm" : "sm", icon: destKind() === v ? "check" : "", attrs: `data-choice="dest:${v}"` });
  const opt = (title, good, weak, b, frames) => `<section class="card pad stack"><div class="row wrap"><h2 class="grow">${title}</h2>${b}</div><p class="sm"><b>よいところ</b>　${good}</p><p class="sm"><b>弱いところ</b>　${weak}</p><div class="frames">${frames}</div></section>`;
  const composer = (kind, view) => as(view, "t3", () => `<div class="composer" style="margin:0"><div>${destHtml(kind)}</div><div class="muted" style="padding:2px 4px">${view === "new" ? `${AIDE}に新しく頼む` : "続きを書く"}</div></div>`);
  return `<div class="page"><div class="head"><h1>話の見せ方</h1><span class="n">この頁は PoC だけのもの</span><div class="end">${btn("結果を届かせてみる", { icon: "bell", act: "arrive" })}</div></div>
  <p class="lede">会話はスレッド（話）が単位になる。案を並べた。「この案で使う」を押すと、左の会話（スマホでは下から開く会話）がその案に替わる。</p>
  ${opt("一覧から入る（おすすめ）", "読む幅が広い。スマホとデスクトップで同じ動きになる。ほかの話の待ちは、戻るボタンの数で分かる。", "話を行き来するとき、一覧を一度挟む。", pickConv("drill"), frame("一覧", as("list", null, () => convHtml("drill"))) + frame("話の中", as("thread", "t2", () => convHtml("drill"))))}
  ${opt("一覧を横に置く", "話の行き来が一押しで済み、届いたものがいつも見える。", "会話の幅が要り、右の画面が狭くなる。スマホでは「一覧から入る」と同じになる。", pickConv("side"), `<figure class="tryframe wide" inert><div class="fr">${as("thread", "t1", () => convHtml("side"))}</div><figcaption class="xs muted">一覧と話の中</figcaption></figure>`)}
  ${opt("一続きに並べて、開く", "いまの一本の会話に近く、時の流れが見える。", "長い話が間に挟まり、どこに送るのかが並びから読み取りにくい。待ちがまとまらない。", pickConv("stream"), frame("開いている話だけが広がる", as("thread", "t3", () => convHtml("stream"))))}
  <section class="card pad stack"><h2>送り先の示し方</h2><div class="frames"><div class="stack"><div class="row"><b class="grow">切り替え（おすすめ）</b>${pickDest("seg")}</div><div class="stack" inert>${composer("seg", "thread")}${composer("seg", "new")}</div><p class="sm muted">二つの行き先がどちらもいつも見え、一押しで替わる。</p></div><div class="stack"><div class="row"><b class="grow">札</b>${pickDest("chip")}</div><div class="stack" inert>${composer("chip", "thread")}${composer("chip", "new")}</div><p class="sm muted">題が長く読める。押すと一覧が開き、どの話にも送り先を替えられる。</p></div></div></section></div>`;
}
