const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const root = document.documentElement;
const esc = (x) => String(x).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const i = (n) => `<svg class="i" aria-hidden="true"><use href="#i-${n}"/></svg>`;
const face = (n) => `<svg class="face" width="${n}" height="${n}" aria-hidden="true"><use href="#aide-face"/></svg>`;
const badge = (text, tone = "", icon = "", x = "") => `<span class="badge ${tone} ${x}">${icon ? i(icon) : ""}${text}</span>`;
const state = (k) => badge(ST[k][0], ST[k][1], ST[k][2], ST[k][3] ?? "");
const WG = { all: "すべて", work: "仕事", home: "個人" };
const wgMark = (w) => (w ? `<span class="row" style="gap:5px"><span class="dot ${w}"></span>${WG[w]}</span>` : "");
const SRC = { said: badge("あなたが言った", "accent", "quote"), confirmed: badge("確かめた", "ok", "check"), guess: badge("推し量り", "line") };
const avatar = (p, x = "") => `<span class="avatar ${p.org === 1 ? "org" : ""} ${x}" style="--h:${p.h}" aria-hidden="true">${p.i}</span>`;
const btn = (label, { icon, v = "", act = "", attrs = "" } = {}) => `<button type="button" class="btn ${v}" ${act ? `data-act="${act}"` : ""} ${attrs}>${icon ? i(icon) : ""}${label ? `<span>${label}</span>` : ""}</button>`;
const ib = (icon, label, act = "", x = "ghost sm") => `<button type="button" class="btn icon ${x}" aria-label="${label}" title="${label}" ${act ? `data-act="${act}"` : ""}>${i(icon)}</button>`;
const select = (label, opts) => `<span class="select"><select aria-label="${label}"><option>${label}：すべて</option>${opts.map((o) => `<option>${o}</option>`).join("")}</select>${i("chevron-down")}</span>`;
const search = (ph = "探す") => `<label class="search">${i("search")}<input class="input" type="search" placeholder="${ph}" aria-label="${ph}" data-search></label>`;
const segs = (name, items, at) => `<div class="segs" role="group" aria-label="${name}">${items.map(([href, label, icon]) => `<a class="seg" href="#${href}" ${href === at ? 'aria-current="page"' : ""}>${icon ? i(icon) : ""}${label}</a>`).join("")}</div>`;
const ask = (what, label = "会話に添える") => `<button type="button" class="btn icon ghost sm" aria-label="${label}" title="${label}" data-act="attach" data-what="${what}">${i("message-plus")}</button>`;
const inWg = (x) => !x.wg || wg === "all" || x.wg === wg;
const acts = (html) => `<div class="acts m-only">${html}</div>`;
const empty = (t) => `<div class="empty">${t}</div>`;
const pic = (n) => `<svg viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" aria-hidden="true">${PIC[n]}</svg>`;

let wg = "all";
const sel = {};
const answered = {};

/* ---------- the conversation ---------- */
const ASKS = { mail: "松本 由紀さんにメールを返す", question: "荷物の量は、どれに近いですか", start: "セッションを始めて、仕事を委ねる", definition: "サービスの定義を変える", cut: "途中で切れた依頼" };
const FROM = { mail: "Google、4 分前", question: "手代からの質問、12 分前", start: "Adroit、2 時間前", definition: "サービス定義、昨日", cut: "昨日 23:10" };
const APPROVALS = {
  mail: () => `<article class="ask" data-ask="mail"><header>${i("shield-check")}<b>確認 #104</b><span>Google（work@example.com）</span><span class="gap"></span><span>4 分前</span></header><div class="in"><h3>松本 由紀さんにメールを返す</h3><dl class="kv"><dt>宛先</dt><dd>matsumoto@example.com</dd><dt>件名</dt><dd>Re: 設計レビューの論点</dd><dt>本文</dt><dd>松本さん<br>論点を三つに絞りました。集計の単位、締めの時刻、過去分の扱いです。資料は共有ドキュメントに置いています。</dd></dl><p class="sm muted">外の人に届くので、あなたに確かめる。</p><details><summary>${i("chevron-right")}元のデータ</summary><pre class="code">gmail.send_message
{ "to": "matsumoto@example.com", "subject": "Re: 設計レビューの論点" }</pre></details></div><footer>${btn("承認", { v: "primary", icon: "check", act: "answer", attrs: 'data-a="承認した"' })}${btn("却下", { act: "answer", attrs: 'data-a="却下した"' })}<span class="gap"></span>${btn("今後は常に許可", { v: "ghost", act: "answer", attrs: 'data-a="今後は常に許可にした"' })}</footer></article>`,
  question: () => `<article class="ask" data-ask="question"><header>${i("message-circle")}<b>確認 #105</b><span>手代からの質問</span><span class="gap"></span><span>12 分前</span></header><div class="in"><h3>荷物の量は、どれに近いですか</h3><p class="sm">「引っ越し業者 3 社の見積もりを比べ、候補を 1 社に絞る」の続きに要る。</p><div class="opts"><label class="opt"><input type="radio" name="q" checked><span><b>段ボール 60 箱くらい（おすすめ）</b><small>前回の見積もりと同じ量で比べる</small></span></label><label class="opt"><input type="radio" name="q"><span><b>段ボール 80 箱くらい</b><small>本が増えた分を足す</small></span></label><label class="opt"><input type="radio" name="q"><span><b>訪問で見積もってもらう</b><small>3 社に訪問の日を頼む</small></span></label></div><label class="field"><span class="label">自分の言葉で答える</span><input class="input" placeholder="例：60 箱と、自転車が 2 台"></label></div><footer>${btn("答える", { v: "primary", icon: "send-2", act: "answer", attrs: 'data-a="reply"' })}${btn("ここでは答えない", { v: "ghost", act: "answer", attrs: 'data-a="ここでは答えなかった"' })}</footer></article>`,
  start: () => `<article class="ask" data-ask="start"><header>${i("shield-check")}<b>確認 #103</b><span>Adroit（このマシン）</span><span class="gap"></span><span>2 時間前</span></header><div class="in"><h3>セッションを始めて、仕事を委ねる</h3><dl class="kv"><dt>タスク</dt><dd><a href="#delegated">経費精算の CSV 取り込みを、新しい書式に合わせて直す</a></dd><dt>ディレクトリ</dt><dd class="mono">~/src/expense-import</dd><dt>頼み</dt><dd>新しい書式の見本に合わせて列の対応を直し、テストを足す。</dd><dt>仕上げ</dt><dd>レビュー、PR、CI。マージはしない</dd></dl></div><footer>${btn("承認", { v: "primary", icon: "check", act: "answer", attrs: 'data-a="承認した"' })}${btn("却下", { act: "answer", attrs: 'data-a="却下した"' })}<span class="gap"></span>${btn("今後は常に許可", { v: "ghost", act: "answer", attrs: 'data-a="今後は常に許可にした"' })}</footer></article>`,
  definition: () => `<article class="ask" data-ask="definition"><header>${i("shield-check")}<b>確認 #101</b><span>サービス定義「北星ネット」</span><span class="gap"></span><span>昨日</span></header><div class="in"><h3>サービスの定義を変える</h3><p class="sm">請求の一覧を読む道具が増えたので、足したい。</p><pre class="code"><span>[[tools]]</span>
<span class="del">- name = "get_bill"</span><span class="add">+ name = "list_bills"</span><span class="add">+ reads = true</span></pre><p class="sm muted">許すと定義に書き込み、つないでいるサービスの道具がこのとおりになる。</p></div><footer>${btn("許す", { v: "primary", icon: "check", act: "answer", attrs: 'data-a="許した"' })}${btn("断る", { act: "answer", attrs: 'data-a="断った"' })}</footer></article>`,
  cut: () => `<article class="ask" data-ask="cut"><header>${i("alert-triangle")}<b>途中で切れた依頼</b><span class="gap"></span><span>昨日 23:10</span></header><div class="in"><p>「共有カレンダーに忘年会の候補日を仮押さえして」の途中で止まった。外への呼び出しが終わったか分からない。</p></div><footer>${btn("続ける", { v: "primary", icon: "player-play", act: "answer", attrs: 'data-a="続けるよう伝えた"' })}${btn("やめる", { act: "answer", attrs: 'data-a="取りやめた"' })}</footer></article>`,
};
// Drawn in more than one place at once, so its radios are named for each.
let drawn = 0;
const request = (k) => (answered[k] ? `<article class="ask done" tabindex="-1" data-done="${k}"><header>${i("circle-check")}<b>${ASKS[k]}</b><span class="gap"></span><span>${esc(answered[k])}</span></header></article>` : APPROVALS[k]().replaceAll('name="q"', `name="q${++drawn}"`));
const waiting = () => Object.keys(ASKS).filter((k) => !answered[k]);

/* ---------- the pages ---------- */
const VIEW = { home: "ホーム、11月26日（木）", calendar: "カレンダー、11月26日からの週", tasks: "タスク", delegated: "タスク、委託", routines: "タスク、定期", memos: "メモ", parties: "相手", things: "物", contracts: "契約", memory: "記憶", approvals: "確認", services: "設定、サービス", look: "設定、表示", more: "ほか", threads: "話の見せ方", parts: "部品" };
const TITLE = { home: "11月26日（木）", calendar: "カレンダー", tasks: "タスク", delegated: "タスク", routines: "タスク", memos: "メモ", parties: "相手", things: "物", contracts: "契約", memory: "記憶", approvals: "確認", services: "サービス", look: "表示", more: "ほか", threads: "話の見せ方", parts: "部品" };
const TABS = [["home", "ホーム", "home"], ["calendar", "カレンダー", "calendar"], ["tasks", "タスク", "checklist"], ["memos", "メモ", "notes"], ["parties", "相手", "users"], ["things", "物", "device-laptop"], ["contracts", "契約", "file-certificate"]];
const ASIDE = [["memory", "記憶", "brain"], ["services", "設定", "settings"]];
const TAB_OF = { delegated: "tasks", routines: "tasks", look: "services", approvals: "", threads: "", parts: "", chat: "home", more: "" };

const jobRow = (j) => `<li class="item ${sel.delegated === j.id ? "on" : ""}"><div class="main"><div class="row wrap" style="justify-content:space-between">${state(j.st)}<span class="xs muted">${j.when}</span></div><button type="button" class="pick t" data-sel="delegated:${j.id}" aria-pressed="${sel.delegated === j.id}">${j.t}</button>${j.pct !== undefined ? `<div class="prog"><div class="meter" style="--bar:var(--info)"><i style="width:${j.pct}%"></i></div><span>${j.pct}%</span><span class="cut">${j.step}</span></div>` : `<span class="xs ${j.st === "ask" ? "" : "muted"}">${j.why ?? j.step}</span>`}<div class="sub">${wgMark(j.wg)}<span class="row" style="gap:4px">${i(j.to === "Adroit" ? "terminal-2" : "robot").replace('class="i"', 'class="i" style="width:13px;height:13px"')}${j.to}</span>${j.due ? `<span>期日 ${j.due}</span>` : ""}${j.pri ? badge("優先度 高", "danger", "flag") : ""}</div></div>${j.st === "ask" ? btn("答える", { v: "soft sm", act: "answerjob", attrs: `data-job="${j.id}"` }) : ""}</li>`;

const taskRow = (t) => `<li class="item ${sel.tasks === t.t ? "on" : ""}"><button type="button" class="circle" aria-pressed="${ticked.has(t.t)}" aria-label="${t.t}を済にする" data-act="check" data-what="${t.t}">${i("check")}</button><div class="main"><button type="button" class="pick t" ${t.job ? `data-go="delegated" data-job="${t.job}" data-act="openjob"` : `data-open="tasks:${t.t}"`}>${t.t}</button><div class="sub">${t.pri ? badge("優先度 高", "danger", "flag") : ""}${t.st ? state(t.st) : ""}${wgMark(t.wg)}${t.due ? `<span>${t.due}</span>` : ""}${t.note ? `<span>${t.note}</span>` : ""}${(t.labels ?? []).map((l) => `<span>#${l}</span>`).join("")}</div></div><span class="badge ${t.who === AIDE ? "aide" : t.who === "委託" ? "line" : ""}">${t.who}</span></li>`;

const counts = () => Object.keys(ST).map((k) => [k, JOBS.filter((j) => j.st === k && inWg(j)).length]).filter(([, n]) => n);
const taskHead = (at) => `<div class="head"><h1>タスク</h1><span class="n">${at === "delegated" ? `${JOBS.filter(inWg).length} 件` : at === "routines" ? "3 件" : `${TASKS.flatMap(([, ts]) => ts).filter(inWg).length} 件`}</span><div class="end d-only">${segs("種類", [["tasks", "一度きり"], ["routines", "定期"], ["delegated", "委託"]], at)}${btn("タスクを足す", { v: "primary", icon: "plus", act: "todo" })}</div></div>`;
const taskActs = (at) => acts(`${segs("種類", [["tasks", "一度きり"], ["routines", "定期"], ["delegated", "委託"]], at)}<span class="gap"></span>${ib("plus", "タスクを足す", "todo", "primary")}`);

const dbar = (key, kind, more = "") => `<div class="dbar"><button type="button" class="btn icon ghost back" aria-label="一覧へ戻る" data-act="close" data-key="${key}">${i("chevron-left")}</button><span class="sm muted grow cut">${kind}</span>${more}<button type="button" class="btn icon ghost sm close" aria-label="閉じる" data-act="close" data-key="${key}">${i("x")}</button></div>`;
const log = (rows) => `<ul class="log">${rows.map(([t, x]) => `<li><time>${t}</time><div>${x}</div></li>`).join("")}</ul>`;
const recordTools = (name) => `${ask(name)}${ib("pencil", "直す", "todo")}${ib("copy", "呼び名を写す", "todo")}${ib("trash", "消す", "remove")}`;

const P = {
  home: () => `<div class="page"><div class="head"><h1>11月26日（木）</h1><div class="end d-only">${btn("配置を変える", { v: "ghost sm", icon: "layout-grid", act: "todo" })}</div></div>
    <section class="brief" aria-label="今日の要約">${face(40)}<div><p>このあと 11:00 に設計レビューがあります。<br class="d-only">${waiting().length ? `確かめていただきたいことが ${waiting().length} つあります。` : "お待たせしている確認はありません。"}</p><div class="facts"><span>予定 4 件</span><span>あなたの今日のタスク 2 件</span><span>${AIDE}が 2 件を進めている</span></div></div></section>
    <div class="tiles"><div class="col">
      <section class="card"><div class="card-head">${i("calendar")}<h2>今日</h2><a class="btn icon ghost sm" href="#calendar" aria-label="カレンダーへ">${i("chevron-right")}</a></div>
        <div style="padding-bottom:8px">${EVENTS.filter(inWg).map((e) => `<div class="ev ${e.now ? "now" : ""} ${e.past ? "past" : ""}"><time>${e.s}<small>${e.e}</small></time><span class="rule ${e.wg}"></span><div><div class="t" style="font-weight:500">${e.t}</div><div class="xs ${e.now ? "" : "muted"}">${e.sub}</div></div>${e.meet ? btn("入る", { v: e.now ? "primary sm" : "sm", icon: "video", act: "todo", attrs: `aria-label="${e.t}に入る"` }) : "<span></span>"}</div>`).join("")}</div></section>
      <section class="card"><div class="card-head">${i("checklist")}<h2>今日と今週のタスク</h2><a class="btn icon ghost sm" href="#tasks" aria-label="タスクへ">${i("chevron-right")}</a></div><ul>${[...TASKS[0][1], ...TASKS[1][1]].filter(inWg).slice(0, 4).map(taskRow).join("")}</ul></section>
      <section class="card"><div class="card-head">${i("notes")}<h2>最近のメモ</h2><a class="btn icon ghost sm" href="#memos" aria-label="メモへ">${i("chevron-right")}</a></div><div class="mini"><a class="pick t" href="#memos">引っ越し先の採寸</a><span class="xs muted">昨日、個人</span></div></section>
    </div><div class="col">
      <section class="card"><div class="card-head">${i("shield-check")}<h2>確認待ち</h2>${waiting().length ? `<span class="count warn">${waiting().length}</span>` : ""}<a class="btn icon ghost sm" href="#approvals" aria-label="確認へ">${i("chevron-right")}</a></div>
        <div style="padding-bottom:6px">${waiting().slice(0, 3).map((k) => `<div class="mini"><a class="pick t" href="#approvals">${ASKS[k]}</a><span class="xs muted">${FROM[k]}</span></div>`).join("") || empty("いまはない")}${waiting().length > 3 ? `<div class="mini xs muted">ほか ${waiting().length - 3} 件</div>` : ""}</div></section>
      <section class="card"><div class="card-head">${i("activity")}<h2>${AIDE}が進めている</h2><a class="btn icon ghost sm" href="#delegated" aria-label="委託へ">${i("chevron-right")}</a></div>
        ${["running", "ask", "blocked", "queued"].map((k) => JOBS.filter((j) => j.st === k && inWg(j)).slice(0, 2).map((j) => `<div class="mini"><div class="row" style="justify-content:space-between">${state(k)}<span class="xs muted">${j.when}</span></div><button type="button" class="pick" data-act="openjob" data-job="${j.id}">${j.t}</button>${j.pct !== undefined ? `<div class="prog"><div class="meter" style="--bar:var(--info)"><i style="width:${j.pct}%"></i></div><span>${j.pct}%</span><span class="cut">${j.step}</span></div>` : ""}</div>`).join("")).join("")}</section>
      <section class="card"><div class="card-head">${i("clock")}<h2>期限の近い物と契約</h2></div><div class="mini"><button type="button" class="pick t" data-open="things:t2">ワークチェアの保証</button><span class="xs muted">あと 12 日、12月8日</span></div><div class="mini" style="padding-bottom:14px"><a class="pick t" href="#contracts">光回線の更新月</a><span class="xs muted">12月</span></div></section>
    </div></div><div class="m-only" style="justify-content:center">${btn("配置を変える", { v: "ghost", icon: "layout-grid", act: "todo" })}</div></div>`,

  calendar: () => {
    const days = [["木", 26, [["work", 11, 1, "設計レビュー", "会議室 3A"], ["work", 14, .5, "採用の打ち合わせ"], ["home", 18, .5, "保育園の迎え"], ["hold", 15.5, 1, "仮：週報を書く"]], "出社"], ["金", 27, [["work", 10, 1.5, "四半期報告の下読み"], ["home", 17.5, 1, "保育園の迎え"]]], ["土", 28, [["home", 10, 2, "新居の採寸"]]], ["日", 29, []], ["月", 30, [["work", 9.5, .5, "朝会"], ["faint", 13, 1, "予定あり"]]], ["火", 1, [["work", 9.5, .5, "朝会"], ["work", 15, 1, "1on1 グエンさん"]]], ["水", 2, [["work", 9.5, .5, "朝会"]]]];
    return `<div class="page"><div class="head"><h1>11月26日 – 12月2日</h1>${btn("今日", { v: "sm", act: "todo" })}${ib("chevron-left", "前の週", "todo")}${ib("chevron-right", "次の週", "todo")}${ib("refresh", "カレンダーを読み直す", "todo")}<div class="end d-only"><div class="segs" role="group" aria-label="見え方">${["日", "週", "月"].map((u) => `<button type="button" class="seg" aria-pressed="${u === "週"}" data-act="press">${u}</button>`).join("")}</div>${btn("予定を足す", { v: "primary", icon: "plus", act: "todo" })}</div></div>
    <div class="card cal"><div class="dh" style="border-left:0"></div>${days.map(([w, d], n) => `<div class="dh ${n ? "" : "today"} ${n > 2 ? "d3" : ""}">${w}<br><b>${d}</b></div>`).join("")}
    <div class="allday xs muted" style="border-left:0;padding:8px 6px">終日</div>${days.map((d, n) => `<div class="allday ${n > 2 ? "d3" : ""}">${d[3] ? `<button type="button" class="cev flat work">${d[3]}</button>` : ""}${n === 1 ? '<span class="xs muted row" style="gap:4px;padding:2px 4px">' + i("flag").replace('class="i"', 'class="i" style="width:12px"') + "報告の下書き</span>" : ""}</div>`).join("")}
    <div class="hours">${Array.from({ length: 9 }, (_, n) => `<span style="top:${(n + 1) * 56}px">${n + 10}:00</span>`).join("")}</div>
    ${days.map((d, n) => `<div class="col ${n > 2 ? "d3" : ""}">${n ? "" : '<div class="nowline" style="top:108px"></div>'}${d[2].filter((e) => wg === "all" || e[0] === wg || e[0] === "hold").concat(wg === "all" ? [] : d[2].filter((e) => e[0] !== wg && e[0] !== "hold").map((e) => ["faint", e[1], e[2], "予定あり"])).map(([c, s, l, t, p]) => `<button type="button" class="cev ${c}" title="${t}" style="top:${(s - 9) * 56 + 1}px;height:${l * 56 - 2}px"><b class="${l < 1 ? "cut" : ""}">${t}</b>${p ? `<span class="muted">${p}</span>` : ""}</button>`).join("")}</div>`).join("")}</div>
    <ul class="legend"><li><span class="sw" style="--c:var(--wg-work)"></span>work@example.com</li><li><span class="sw" style="--c:var(--wg-home)"></span>home@example.com</li><li><span class="sw" style="--c:var(--pressed)"></span>ワークグループの外（予定あり）</li><li><span class="sw" style="--c:transparent;outline:1.5px dashed var(--control);outline-offset:-1px"></span>${AIDE}の仮押さえ</li></ul></div>${acts(`<div class="segs" role="group" aria-label="見え方">${["日", "3日", "月"].map((u) => `<button type="button" class="seg" aria-pressed="${u === "3日"}" data-act="press">${u}</button>`).join("")}</div><span class="gap"></span>${ib("plus", "予定を足す", "todo", "primary")}`)}`;
  },

  tasks: () => `<div class="split ${sel.tasks ? "open" : ""}"><div class="page">${taskHead("tasks")}<div class="filters">${search()}${select("担当", ["あなた", AIDE, "委託"])}${select("状態", ["まだ", "進行中", "待ち", "済"])}${select("優先度", ["高", "ふつう", "低"])}</div>
    <a class="card row sumcard" href="#delegated" style="padding:10px 14px;color:inherit;text-decoration:none">${i("robot")}<b style="font-weight:500">委ねた仕事</b><span class="row wrap grow">${counts().filter(([k]) => ["ask", "running", "queued", "blocked"].includes(k)).map(([k, n]) => badge(`${ST[k][0]} ${n}`, ST[k][1], ST[k][2], "")).join("")}</span>${i("chevron-right")}</a>
    ${TASKS.map(([g, ts]) => [g, ts.filter(inWg)]).filter(([, ts]) => ts.length).map(([g, ts]) => `<section class="stack" style="gap:8px"><div class="group"><h2>${g}</h2><span class="muted">${ts.length}</span></div><ul class="list">${ts.map(taskRow).join("")}</ul></section>`).join("")}</div>${sel.tasks ? taskDetail() : ""}</div>${sel.tasks ? "" : taskActs("tasks")}`,

  routines: () => `<div class="page">${taskHead("routines")}<ul class="list">${ROUTINES.map((r) => `<li class="item mid"><div class="main"><button type="button" class="pick t" data-act="todo">${r.t}</button><div class="sub">${i("repeat").replace('class="i"', 'class="i" style="width:13px"')}<span>${r.when}</span><span>${r.on ? `次は${r.next}` : "止めている"}</span></div></div><input type="checkbox" class="switch" aria-label="${r.t}を動かす" ${r.on ? "checked" : ""}></li>`).join("")}</ul></div>${taskActs("routines")}`,

  delegated: () => `<div class="split ${sel.delegated ? "open" : ""}"><div class="page">${taskHead("delegated")}
    <div class="sumrow" role="group" aria-label="状態で絞る"><button type="button" class="chip" data-act="st" data-st="" aria-pressed="${!sel.st}">すべて ${JOBS.filter(inWg).length}</button>${counts().map(([k, n]) => `<button type="button" class="chip" data-act="st" data-st="${k}" aria-pressed="${sel.st === k}">${i(ST[k][2])}${ST[k][0]} ${n}</button>`).join("")}</div>
    ${[["あなたを待っている", ["ask"]], ["動いている", ["running"]], ["これから", ["queued", "blocked"]], ["終わった", ["done", "failed"]]].map(([g, ks]) => [g, JOBS.filter((j) => ks.includes(j.st) && inWg(j) && (!sel.st || sel.st === j.st))]).filter(([, js]) => js.length).map(([g, js]) => `<section class="stack" style="gap:8px"><div class="group"><h2>${g}</h2><span class="muted">${js.length}</span></div><ul class="list">${js.map(jobRow).join("")}</ul></section>`).join("")}</div>${sel.delegated ? jobDetail() : ""}</div>${sel.delegated ? "" : taskActs("delegated")}`,

  memos: () => `<div class="page"><div class="head"><h1>メモ</h1><span class="n">3 件</span><div class="end d-only">${btn("メモを足す", { v: "primary", icon: "plus", act: "todo" })}</div></div><div class="filters">${search()}${select("ラベル", ["住まい", "執筆", "ai-note"])}</div><ul class="list">${[["引っ越し先の採寸", "玄関の幅は 78 cm。冷蔵庫は横にしないと入らない。", "昨日", "home", "住まい"], ["設計レビューの論点", "集計の単位、締めの時刻、過去分の扱い。", "11月24日", "work", "設計"], ["読みたい本", "日本語組版の考え方の次に、欧文の本を 1 冊。", "11月20日", "home", "読書"]].filter(([, , , w]) => inWg({ wg: w })).map(([t, b, d, w, l]) => `<li class="item"><div class="main"><button type="button" class="pick t" data-act="todo">${t}</button><span class="sm muted">${b}</span><div class="sub">${wgMark(w)}<span>#${l}</span><span>${d}</span></div></div>${ask(t)}</li>`).join("")}</ul></div>${acts(`<span class="gap"></span>${btn("メモを足す", { v: "primary", icon: "plus", act: "todo" })}`)}`,

  parties: () => `<div class="split ${sel.parties ? "open" : ""}"><div class="page"><div class="head"><h1>相手</h1><span class="n">${[...PEOPLE, ...ORGS].filter(inWg).length} 件</span><div class="end d-only">${btn("相手を足す", { v: "primary", icon: "plus", act: "todo" })}</div></div><div class="filters">${search()}${select("種別", ["人", "会社", "チーム", "グループ"])}</div>
    ${[["人", PEOPLE], ["会社・チーム", ORGS]].map(([g, ps]) => [g, ps.filter(inWg)]).filter(([, ps]) => ps.length).map(([g, ps]) => `<section class="stack" style="gap:8px"><div class="group"><h2>${g}</h2><span class="muted">${ps.length}</span></div><ul class="list">${ps.map((p) => `<li class="item mid ${sel.parties === p.id ? "on" : ""}">${avatar(p)}<div class="main"><button type="button" class="pick t" data-sel="parties:${p.id}" aria-pressed="${sel.parties === p.id}">${p.n}</button><div class="sub"><span>${p.role}</span><span>${p.sub}</span>${p.alias ? `<span>${p.alias}</span>` : ""}${wgMark(p.wg)}</div></div>${p.guess ? badge("推し量り", "line") : ""}<span class="xs muted">${p.seen}</span></li>`).join("")}</ul></section>`).join("")}</div>${sel.parties ? partyDetail() : ""}</div>${sel.parties ? "" : acts(`<span class="gap"></span>${btn("相手を足す", { v: "primary", icon: "plus", act: "todo" })}`)}`,

  things: () => holdings("things"),
  contracts: () => holdings("contracts"),

  memory: () => {
    const at = sel.layer;
    const nav = `<nav class="sidenav ${at ? "" : "index"}" aria-label="記憶"><button type="button" class="nav" data-act="layer" data-layer="dream" ${(at ?? "dream") === "dream" ? 'aria-current="page"' : ""}>${i("moon-stars")}夢</button><div class="h">層</div>${Object.entries(MEM).map(([k, m]) => `<button type="button" class="nav" data-act="layer" data-layer="${k}" ${at === k ? 'aria-current="page"' : ""}>${i(m.icon)}${m.t}<span class="xs">${m.c}</span></button>`).join("")}</nav>`;
    return `<div class="split ${sel.entry ? "open" : ""}">${sel.entry ? "" : nav}${!at || at === "dream" ? dream(!at) : layer(at)}${sel.entry ? entryDetail() : ""}</div>`;
  },

  approvals: () => `<div class="page"><div class="head"><h1>確認</h1><span class="n">あなたを待っているもの</span><div class="end">${btn("入力の控えを書き出す", { v: "ghost sm", icon: "file-text", act: "todo" })}</div></div><p class="lede">会話の中と同じカードで、ここでも答えられる。どこで答えても、ほかの場所の確認も答え済みになる。</p><div style="columns:2 360px;column-gap:16px" class="askgrid">${Object.keys(APPROVALS).map((k) => `<div style="break-inside:avoid;margin-bottom:16px">${request(k)}${THREADS.filter((t) => t.asks?.includes(k)).map((t) => `<button type="button" class="btn ghost sm" style="max-width:100%" data-act="opentalk" data-id="${t.id}">${i("message-circle")}<span class="cut" data-title="${t.id}"></span></button>`).join("")}</div>`).join("")}<div style="break-inside:avoid"><article class="ask done"><header>${i("circle-check")}<b>共有カレンダーに仮押さえを入れる</b><span class="gap"></span><span>Slack で承認した、9:02</span></header></article></div></div></div>`,

  services: () => settings("services", `<div class="head"><h1>サービス</h1><div class="end d-only">${btn("サービスを足す", { v: "primary", icon: "plus", act: "todo" })}</div></div><p class="lede">あなたのアカウントや端末をつなぐ。つないだものは秘書が使い、カレンダー・見張り・チャネルにも入る。</p><div class="filters">${search("名前・アカウントで探す")}</div>
    ${[["手当ての要るもの", SERVICES.filter((s) => s.st === "warn")], ["つながっている", SERVICES.filter((s) => s.st === "ok")], ["はじめからあるもの", SERVICES.filter((s) => s.st === "own")]].map(([g, ss]) => [g, ss.filter(inWg)]).filter(([, ss]) => ss.length).map(([g, ss]) => `<section class="stack" style="gap:8px"><div class="group"><h2>${g}</h2><span class="muted">${ss.length}</span></div><ul class="list">${ss.map((s) => `<li class="item mid wraps"><span class="avatar org" style="background:var(--sunken);color:var(--text-2)">${i(s.icon)}</span><div class="main"><button type="button" class="pick t" data-act="todo">${s.t}</button><div class="sub"><span>${s.a}</span><span>${s.d}</span>${wgMark(s.wg)}</div></div>${s.st === "warn" ? `${badge(s.msg, "warn", "alert-triangle")}${btn("つなぎ直す", { v: "sm", act: "todo" })}` : s.st === "ok" ? badge("つながっている", "ok", "check") : ""}</li>`).join("")}</ul></section>`).join("")}`, btn("サービスを足す", { v: "primary", icon: "plus", act: "todo" })),

  look: () => settings("look", `<div class="head"><h1>表示</h1></div><p class="lede">この端末での見え方。テーマはこのブラウザに残り、開いているほかのタブにも効く。</p>
    <section class="card pad stack"><h2>テーマ</h2><div class="segs" role="radiogroup" aria-label="テーマ" style="align-self:flex-start">${themeSegs(true)}</div><p class="sm muted">システムを選ぶと、端末のライトとダークに従い、端末で切り替えるとその場で替わる。</p></section>
    <section class="card pad stack"><h2>アクセントの色</h2>${choice("accent", [["green", "深緑"], ["indigo", "夜藍"], ["plum", "葡萄"]])}<h2>密度</h2>${choice("density", [["compact", "詰める"], ["normal", "ふつう"], ["roomy", "ゆったり"]])}<h2>角丸</h2>${choice("radius", [["small", "小"], ["medium", "中"], ["large", "大"]])}<h2>見出しの書体</h2>${choice("heading", [["gothic", "ゴシック"], ["mincho", "明朝"]])}</section>`),

  more: () => `<div class="page"><ul class="list">${[["parties", "users", "相手", "人と会社・チーム・グループ"], ["things", "device-laptop", "物", "機器、期限の近い順"], ["contracts", "file-certificate", "契約", "ライセンス・サブスクリプション・回線や保険"], ["memory", "brain", "記憶", "夢、保留中の判断 3"], ["approvals", "shield-check", "確認", `あなたを待っているもの ${waiting().length}`], ["services", "settings", "設定", "ワークグループ、秘書、つながり"], ["look", "palette", "表示", "テーマ、色、密度"]].map(([h, ic, t, d]) => `<li class="item mid">${i(ic)}<div class="main"><a class="pick t" href="#${h}">${t}</a><div class="sub">${d}</div></div>${i("chevron-right")}</li>`).join("")}</ul></div>`,

  threads: () => threadsPage(),

  parts: () => parts(),
};

const taskDetail = () => {
  const t = TASKS.flatMap(([, ts]) => ts).find((x) => x.t === sel.tasks);
  return `<aside class="detail" aria-label="${t.t}">${dbar("tasks", "タスク", recordTools(t.t))}<div class="dscroll"><div class="stack" style="gap:10px"><h1>${t.t}</h1><div class="row wrap">${t.pri ? badge("優先度 高", "danger", "flag") : ""}${badge("まだ")}<span class="sm muted row">${wgMark(t.wg)}</span></div></div><dl class="kv"><dt>担当</dt><dd>${t.who}</dd><dt>期日</dt><dd>${t.due || "決めていない"}</dd><dt>関わるもの</dt><dd><a class="chip" href="#parties" data-open="parties:p1">${i("user")}松本 由紀</a></dd></dl><div class="row wrap">${btn("済にする", { v: "primary", icon: "check", act: "todo" })}${btn(`${AIDE}に任せる`, { icon: "arrow-right", act: "todo" })}</div><section class="sec"><h3>備考</h3><div class="prose"><p>論点は、集計の単位、締めの時刻、過去分の扱い。返事の下書きはメールの下書きにある。</p></div></section><section class="sec"><h3>経過</h3>${log([["今日 10:41", `${AIDE}が返事を下書きした`], ["昨日 18:20", "メールの見張りから足された"]])}<label class="row"><input class="input" placeholder="経過を書く" aria-label="経過を書く">${btn("書く", { act: "todo" })}</label></section><section class="sec"><h3>ここにつながる記憶</h3><ul class="list"><li class="item"><div class="main"><button type="button" class="pick" data-open="memory:m12">松本さんは、資料は PDF より共有ドキュメントを好む</button><div class="sub">${SRC.confirmed}</div></div></li></ul></section></div></aside>`;
};
const jobDetail = () => {
  const j = JOBS.find((x) => x.id === sel.delegated);
  return `<aside class="detail" aria-label="${j.t}">${dbar("delegated", "委ねた仕事", recordTools(j.t))}<div class="dscroll"><div class="stack" style="gap:10px"><div>${state(j.st)}</div><h1>${j.t}</h1>${j.pct !== undefined ? `<div class="stack" style="gap:6px"><div class="meter" style="--bar:var(--info)"><i style="width:${j.pct}%"></i></div><div class="row sm muted"><span>${j.pct}%</span><span>${j.step}</span><span class="gap"></span><span>${j.when}</span></div></div>` : `<p class="sm muted">${j.why ?? j.step}</p>`}</div>
  ${j.st === "ask" ? request(j.id === "j1" ? "question" : "start") : ""}${j.st === "failed" ? `<div class="callout danger">${i("alert-triangle")}<span>写真の置き場所に届かなかった。つなぎ先を直してから並びに戻すと、また動く。</span></div>` : ""}
  ${j.st === "blocked" ? `<div class="callout">${i("lock")}<span>先の仕事が済むと、ひとりでに始まる。<button type="button" class="pick" style="position:relative;text-decoration:underline" data-sel="delegated:j3">四半期報告の数字を、基幹の集計と突き合わせる</button></span></div>` : ""}
  ${j.st === "running" ? `<section class="sec"><h3>進み具合</h3><ul class="stack" style="gap:8px">${[["circle-check", "ok", "集計の取り出し"], ["circle-check", "ok", "列の対応を決める"], ["refresh", "info", j.step], ["clock", "muted", "まとめを書く"]].map(([ic, c, t]) => `<li class="row sm" style="color:var(--${c})">${i(ic)}<span style="color:var(--text)">${t}</span></li>`).join("")}</ul></section>` : ""}
  <dl class="kv">${THREADS.filter((t) => t.job === j.id).map((t) => `<dt>元の話</dt><dd><button type="button" class="chip" data-act="opentalk" data-id="${t.id}">${i("message-circle")}<span class="cut" data-title="${t.id}"></span></button></dd>`).join("")}<dt>委託先</dt><dd>${j.to}</dd><dt>ワークグループ</dt><dd>${wgMark(j.wg)}</dd><dt>期日</dt><dd>${j.due ?? "決めていない"}</dd><dt>優先度</dt><dd>${j.pri ? "高" : "ふつう"}</dd><dt>渡す道具</dt><dd>ファイルを読む・書く、ウェブを探す</dd>${j.to === "Adroit" ? '<dt>ディレクトリ</dt><dd class="mono">~/src/report</dd>' : ""}</dl>
  <section class="sec"><h3>頼み</h3><div class="prose"><p>${j.t}。食い違いがあれば、行と理由を一覧にする。元のデータは変えない。</p></div></section><section class="sec"><h3>経過</h3>${log([["今日 9:40", "始まった"], ["今日 9:12", `${AIDE}が委ねた`]])}</section>
  <div class="row wrap">${j.to === "Adroit" ? btn("セッションを開く", { icon: "external-link", act: "todo" }) : ""}${j.st === "done" || j.st === "failed" ? btn("並びに戻す", { icon: "arrow-back-up", act: "todo" }) : btn("やめる", { v: "danger", icon: "x", act: "remove" })}</div></div></aside>`;
};
const partyDetail = () => {
  const p = [...PEOPLE, ...ORGS].find((x) => x.id === sel.parties);
  const members = PEOPLE.filter((x) => x.org === p.n);
  return `<aside class="detail" aria-label="${p.n}">${dbar("parties", "相手", recordTools(p.n))}<div class="dscroll"><div class="row" style="gap:16px;align-items:center">${avatar(p, "xl")}<div class="stack" style="gap:4px"><h1>${p.n}</h1><div class="sm muted">${p.sub}${p.org && p.org !== 1 ? `、${p.org}` : ""}</div><div class="row wrap">${badge(p.role, "accent")}${p.guess ? badge("推し量り", "line") : ""}<span class="xs muted">${wgMark(p.wg)}</span></div></div></div>
  <dl class="kv">${p.alias ? `<dt>ほかの名前</dt><dd>${p.alias}</dd><dt>誕生日</dt><dd>12月</dd>` : ""}<dt>最後に出た日</dt><dd>${p.seen}</dd><dt>足した人</dt><dd>${AIDE}</dd></dl>
  ${members.length ? `<section class="sec"><h3>属する人</h3><ul class="list">${members.map((m) => `<li class="item mid">${avatar(m)}<div class="main"><button type="button" class="pick t" data-sel="parties:${m.id}">${m.n}</button><div class="sub">${m.role}、${m.sub}</div></div></li>`).join("")}</ul></section>` : `<section class="sec"><h3>関係</h3><ul class="list"><li class="item mid">${i("user")}<div class="main"><span class="t">あなたの${p.role}</span></div>${badge("確かめた", "ok", "check")}</li>${p.id === "p1" ? `<li class="item mid">${avatar(PEOPLE[1])}<div class="main"><button type="button" class="pick t" data-sel="parties:p2">田所 健</button><div class="sub">逆から見ると部下</div></div>${badge("推し量り", "line")}</li>` : ""}</ul></section>
  <section class="sec"><h3>アカウント</h3><ul class="list"><li class="item mid">${i("mail")}<div class="main"><span class="t">person@example.com</span><div class="sub">メール</div></div>${badge("確かめた", "ok", "check")}</li><li class="item mid">${i("brand-slack")}<div class="main"><span class="t">@${p.alias}</span><div class="sub">青葉システム</div></div>${badge("推し量り", "line")}${btn("同じ人", { v: "sm", act: "todo" })}</li></ul></section>`}
  <section class="sec"><h3>つながり</h3><div class="row wrap"><a class="chip" href="#calendar">${i("calendar")}設計レビュー</a><a class="chip" href="#tasks">${i("checklist")}設計レビューの返事</a><a class="chip" href="#memory" data-open="memory:m12">${i("brain")}資料の好み</a></div></section>
  <section class="sec"><h3>備考</h3><div class="prose"><p>資料は PDF より共有ドキュメントを好む。返事は早いが、夕方以降は翌朝になる。</p></div></section></div></aside>`;
};
function holdings(kind) {
  const list = (kind === "things" ? THINGS : CONTRACTS).filter(inWg).filter((x) => !sel[`${kind}cat`] || x.cat === sel[`${kind}cat`]);
  const all = (kind === "things" ? THINGS : CONTRACTS).filter(inWg);
  const cats = [...new Set(all.map((x) => x.cat))];
  const cards = (sel[`${kind}v`] ?? (kind === "things" ? "card" : "list")) === "card";
  const open = sel[kind];
  const name = kind === "things" ? "物" : "契約";
  const tags = (x) => `${x.st ? badge(x.st, "info", "clock") : ""}${x.old ? badge("古いかもしれない", "warn", "alert-triangle") : ""}${x.due ? badge(x.due, x.warn || x.id === "t2" ? "warn" : "", "clock") : ""}`;
  const lead = (x) => (x.pic ? `<span class="thumb">${pic(x.pic)}</span>` : avatar({ ...x, org: 1 }));
  const viewSegs = `<div class="segs" role="group" aria-label="並べ方"><button type="button" class="seg" aria-pressed="${!cards}" data-act="hv" data-k="${kind}" data-v="list">${i("list")}リスト</button><button type="button" class="seg" aria-pressed="${cards}" data-act="hv" data-k="${kind}" data-v="card">${i("layout-cards")}カード</button></div>`;
  return `<div class="split ${open ? "open" : ""}"><div class="page"><div class="head"><h1>${name}</h1><span class="n">${all.length} 件</span><div class="end d-only">${viewSegs}${btn(`${name}を足す`, { v: "primary", icon: "plus", act: "todo" })}</div></div><div class="filters">${search()}${kind === "things" ? select("状態", ["使っている", "注文中", "貸している"]) : ""}<label class="check"><input type="checkbox">${kind === "things" ? "手放した物も" : "終わった契約も"}</label></div>
  <div class="sumrow" role="group" aria-label="分類">${["", ...cats].map((c) => `<button type="button" class="chip" data-act="cat" data-k="${kind}" data-v="${c}" aria-pressed="${(sel[`${kind}cat`] ?? "") === c}">${c || "すべて"} ${c ? all.filter((x) => x.cat === c).length : all.length}</button>`).join("")}</div>
  ${cards ? `<div class="cards">${list.map((x) => `<article class="card thing"><div class="pic">${x.pic ? pic(x.pic) : `<span class="avatar org xl" style="--h:${x.h}">${x.i}</span>`}</div><div class="b"><button type="button" class="pick t" data-sel="${kind}:${x.id}">${x.t}</button><span class="xs muted">${[x.sub, x.place, x.price].filter(Boolean).join("、")}</span><div class="foot">${tags(x)}<span class="xs muted">${wgMark(x.wg)}</span></div></div></article>`).join("")}</div>` : `<ul class="list">${list.map((x) => `<li class="item mid ${open === x.id ? "on" : ""}">${lead(x)}<div class="main"><button type="button" class="pick t" data-sel="${kind}:${x.id}">${x.t}</button><div class="sub"><span>${x.sub}</span>${x.price ? `<span>${x.price}</span>` : ""}${wgMark(x.wg)}</div><div class="row wrap">${tags(x)}</div></div></li>`).join("")}</ul>`}</div>${open ? holdingDetail(kind, all.concat(kind === "things" ? THINGS : CONTRACTS).find((x) => x.id === open), lead) : ""}</div>${open ? "" : acts(`${viewSegs}<span class="gap"></span>${ib("plus", `${name}を足す`, "todo", "primary")}`)}`;
}
const holdingDetail = (kind, x) => `<aside class="detail" aria-label="${x.t}">${dbar(kind, kind === "things" ? "物" : "契約", recordTools(x.t))}<div class="dscroll">${x.pic ? `<div class="thing card" style="box-shadow:none"><div class="pic" style="aspect-ratio:16/8;border:0">${pic(x.pic)}</div></div>` : ""}<div class="stack" style="gap:8px"><h1>${x.t}</h1><div class="row wrap">${badge(x.cat)}${x.st ? badge(x.st, "info", "clock") : ""}<span class="xs muted">${wgMark(x.wg)}</span></div></div>${x.old ? `<div class="callout warn">${i("alert-triangle")}<span class="grow">古くなったかもしれない（11月20日）：半年、確かめていない</span>${btn("確かめた", { v: "sm", act: "todo" })}</div>` : ""}<dl class="kv"><dt>金額</dt><dd>${x.price || "書いていない"}</dd>${kind === "things" ? `<dt>置き場所</dt><dd>${x.place || "まだ"}</dd><dt>手に入れた</dt><dd>${x.sub}</dd><dt>製造番号</dt><dd class="mono">EX-0042-1187</dd>` : `<dt>提供元</dt><dd>${x.sub}</dd><dt>支払い方法</dt><dd>クレジットカード</dd>`}</dl>${x.due ? `<section class="sec"><h3>期限</h3><ul class="list"><li class="item mid">${i("clock")}<div class="main"><span class="t">${x.due}</span><div class="sub">2 週間前になると、タスクになる</div></div></li></ul></section>` : ""}<section class="sec"><h3>つながり</h3><div class="row wrap"><a class="chip" href="#tasks">${i("checklist")}椅子の座面の交換部品を注文する</a></div></section><section class="sec"><h3>備考</h3><div class="prose"><p>座面の品番は背の裏の札にある。</p></div></section></div></aside>`;

function dream(index) {
  const d = [[3, 0, 0, 1], [0, 0, 0, 0], [2, 1, 0, 0], [5, 0, 1, 2], [1, 1, 0, 0], [0, 2, 0, 1], [4, 0, 0, 0], [2, 0, 1, 0], [0, 0, 0, 0], [6, 1, 0, 1], [1, 0, 0, 3], [3, 2, 0, 0], [2, 0, 1, 1], [4, 1, 0, 0]];
  return `<div class="page ${index ? "page-with-index" : ""}"><div class="head"><button type="button" class="btn icon ghost back m-only" style="display:none" data-act="layer" data-layer="" aria-label="記憶の一覧へ戻る">${i("chevron-left")}</button><h1>夢</h1><div class="end">${btn("今すぐ夢を見る", { icon: "refresh", act: "todo" })}</div></div><p class="lede">次は明日 4:00。気づきが 30 件たまれば早めに見る。いま待っている気づき 3 件。</p>
  <section class="card pad stack"><h2>夢ごとの変化（この 2 週間）</h2><ul class="legend">${["足した", "直した", "置き換えた", "忘れた"].map((t, n) => `<li><span class="sw" style="--c:var(--c${n + 1})"></span>${t}</li>`).join("")}</ul><div class="bars">${d.map((v, n) => `<button type="button" class="${n === 13 ? "on" : ""}" aria-label="11月${n + 12}日：足した ${v[0]}、直した ${v[1]}、置き換えた ${v[2]}、忘れた ${v[3]}">${v.map((x, k) => (x ? `<i style="--c:var(--c${k + 1});height:${x * 13}px"></i>` : "")).join("")}</button>`).join("")}</div><div class="axis" aria-hidden="true">${d.map((_, n) => `<span>${n + 12}</span>`).join("")}</div></section>
  <section class="card pad stack"><div class="row"><h2 class="grow">最後の夢、11月25日 4:02</h2>${ib("chevron-left", "前の夜", "todo")}${ib("chevron-right", "次の夜", "todo")}</div><p class="sm muted">変更の案 5 件を当て、気づきを 6 件捨てた</p><ul class="stack" style="gap:8px">${[["足した", "ok", "大野先生は、連絡帳より電話のほうが早い", "別々の日に 2 度あった"], ["直した", "info", "出張の精算は、月末に一度で出す", "あなたが 11月24日に言い直した"], ["捨てた", "", "椅子の部品の値段を調べた", "記録にすでにある"]].map(([k, c, t, w]) => `<li class="row" style="align-items:flex-start">${badge(k, c)}<div class="grow"><div>${t}</div><div class="xs muted">${w}</div></div></li>`).join("")}</ul></section></div>`;
}
function layer(k) {
  const m = MEM[k];
  const es = ENTRIES[k].filter(inWg);
  const groups = [...new Set(es.map((e) => e.g ?? ""))];
  return `<div class="page"><div class="head"><button type="button" class="btn icon ghost back m-only" data-act="layer" data-layer="" aria-label="記憶の一覧へ戻る">${i("chevron-left")}</button><h1 style="display:block">${m.t}</h1><span class="n">${m.c}</span><div class="end">${btn("読む・書く・経緯", { v: "sm", icon: "pencil", act: "todo" })}</div></div><p class="lede">${m.note}</p>${k === "core" ? '<div class="stack" style="gap:6px"><div class="meter"><i style="width:3%"></i></div><span class="xs muted">大きさ 4 / 150 行</span></div>' : ""}
  ${groups.map((g) => `<section class="stack" style="gap:8px">${g ? `<div class="group"><h2>${g}</h2><span class="muted">${es.filter((e) => (e.g ?? "") === g).length}</span></div>` : ""}<ul class="list">${es.filter((e) => (e.g ?? "") === g).map((e) => `<li class="item ${sel.entry === e.id ? "on" : ""}">${e.time ? `<span class="xs muted" style="padding-top:2px">${e.time}</span>` : ""}<div class="main"><button type="button" class="pick t" data-entry="${e.id}" aria-pressed="${sel.entry === e.id}" ${k === "forgotten" ? 'style="text-decoration:line-through;color:var(--muted)"' : ""}>${e.t}</button><div class="sub">${SRC[e.src]}${e.ev ? `<span class="row" style="gap:6px"><span class="meter" style="width:40px"><i style="width:${(e.ev[0] / e.ev[1]) * 100}%"></i></span>${e.sub}</span>` : e.sub ? `<span>${e.sub}</span>` : ""}${e.against ? badge("あなたの言葉と食い違う", "warn", "alert-triangle") : ""}${wgMark(e.wg)}</div></div>${ask(e.t, k === "pending" ? "答える" : k === "forgotten" ? "戻すよう頼む" : "会話に添える")}</li>`).join("")}</ul></section>`).join("") || empty("このワークグループのものはまだない")}</div>`;
}
const entryDetail = () => {
  const k = Object.keys(ENTRIES).find((x) => ENTRIES[x].some((e) => e.id === sel.entry));
  const e = ENTRIES[k].find((x) => x.id === sel.entry);
  return `<aside class="detail" aria-label="${MEM[k].t}の詳細">${dbar("entry", MEM[k].t, `${ask(e.t)}${ib("pencil", "読む・書く", "todo")}${ib("history", "経緯", "todo")}`)}<div class="dscroll"><div class="stack" style="gap:10px"><h1>${e.t}</h1><div class="row wrap">${SRC[e.src]}${badge(MEM[k].t, "line", MEM[k].icon)}${k === "episodic" ? badge("夢待ち", "", "moon-stars") : ""}${e.against ? badge("あなたの言葉と食い違う", "warn", "alert-triangle") : ""}<span class="xs muted">${wgMark(e.wg)}</span></div></div>
  ${e.ev ? `<section class="sec"><h3>根拠</h3><div class="row"><div class="meter grow"><i style="width:${(e.ev[0] / e.ev[1]) * 100}%"></i></div><span class="sm">${e.ev[0]} / 要る ${e.ev[1]}</span></div><div class="row wrap">${btn("そのとおり", { v: "soft", icon: "check", act: "todo" })}${btn("違う", { act: "todo" })}</div></section>` : ""}
  <section class="sec"><h3>要約</h3><div class="prose"><p>${e.t}。${e.src === "guess" ? "届いたものから推し量ったもので、まだ確かめていない。" : "会話の中であなたが言ったことから書き留めた。"}</p></div></section>
  <section class="sec"><h3>出どころ</h3><div class="card pad" style="box-shadow:none"><dl class="kv"><dt>どこ</dt><dd class="row" style="gap:6px">${i(e.src === "guess" ? "mail" : "message-circle")}${e.src === "guess" ? "メール、home@example.com" : "コンソールの会話"}</dd>${e.src === "guess" ? "<dt>差出人</dt><dd>大野 朝子</dd><dt>件名</dt><dd>金曜のお迎えについて</dd><dt>見張りの判断</dt><dd>取り次いだ。予定に関わる</dd>" : ""}<dt>日時</dt><dd>11月26日 9:15</dd><dt>抜粋</dt><dd style="border-left:2px solid var(--line);padding-left:10px;color:var(--text-2)">金曜は 18 時のお迎えに変わったとうかがいました。</dd></dl><div style="margin-top:12px">${btn(e.src === "guess" ? "メールを開く" : "発言を開く", { v: "sm", icon: "external-link", act: "todo" })}</div></div></section>
  <section class="sec"><h3>リンク</h3><div class="row wrap"><a class="chip" href="#parties" data-open="parties:p5">${i("user")}大野 朝子</a><a class="chip" href="#calendar">${i("calendar")}保育園の迎え</a></div></section>
  <section class="sec"><h3>属性</h3><dl class="kv"><dt>記録した時刻</dt><dd>11月26日 ${e.time ?? "9:15"}</dd><dt>確かさ</dt><dd>${{ said: "確かな事実（あなたが言った）", confirmed: "確かな事実（あなたが確かめた）", guess: "推し量り" }[e.src]}</dd><dt>書いたもの</dt><dd>${e.src === "guess" ? "見張り" : AIDE}</dd></dl></section>
  <section class="sec"><h3>経緯</h3>${log([["11月26日", "気づきに書いた"], ["11月25日", "夢が根拠を数え直した"]])}</section></div></aside>`;
};
const SETTINGS = [["", [["", "ワークグループ", "template"], ["look", "表示", "palette"]]], ["エージェント", [["", "秘書", "id-badge-2"], ["", "夢", "moon-stars"], ["", "見張り", "eye"], ["", "トリアージ", "route"], ["", "見極め", "shield-check"], ["", "手代", "robot"]]], ["モデル", [["", "登録したモデル", "cpu"], ["", "使った量", "chart-bar"]]], ["つながり", [["", "通知", "bell"], ["", "チャネル", "messages"], ["services", "サービス", "plug-connected"], ["", "金庫", "key"]]]];
const settings = (at, body, act = "") => `<div class="split"><nav class="sidenav" aria-label="設定">${SETTINGS.map(([h, xs]) => `${h ? `<div class="h">${h}</div>` : ""}${xs.map(([k, t, ic]) => (k ? `<a class="nav" href="#${k}" ${k === at ? 'aria-current="page"' : ""}>${i(ic)}${t}</a>` : `<button type="button" class="nav" data-act="todo">${i(ic)}${t}</button>`)).join("")}`).join("")}</nav><div class="page">${body}</div></div>${act ? acts(`<span class="gap"></span>${act}`) : ""}`;

function parts() {
  const sw = ["bg", "chat-bg", "surface", "sunken", "raised", "text", "text-2", "muted", "line", "control", "accent", "accent-soft", "aide", "info", "ok", "warn", "danger"];
  return `<div class="page"><div class="head"><h1>部品</h1><span class="n">この見本帳は PoC だけのもの</span></div>
  <section class="card pad stack"><h2>色</h2><div class="swatches">${sw.map((s) => `<div class="swatch"><i style="background:var(--${s})"></i><span>${s}</span></div>`).join("")}</div></section>
  <section class="card pad stack"><h2>文字</h2><h1>見出し：今日のうちに片づけること</h1><h2>小見出し：会議と会議のあいだ</h2><div class="prose"><p>本文は読み物として組む。行間は 1.8、一行は四十字ほどで折り、句読点の並びは詰めすぎない。英数字の <code>2026-11-26</code> や Google も混ぜて見る。</p></div><p class="sm muted">補足：12 分前、個人</p><p class="mono sm">等幅 ~/src/report 0O1lI</p></section>
  <section class="card pad stack"><h2>ボタン</h2><div class="row wrap">${btn("承認", { v: "primary", icon: "check" })}${btn("タスクを開く")}${btn("答える", { v: "soft" })}${btn("配置を変える", { v: "ghost" })}${btn("やめる", { v: "danger" })}${btn("消す", { v: "danger-solid" })}${btn("送っている", { v: "busy", icon: "refresh" })}<button class="btn primary" disabled>使えない</button>${ib("message-plus", "会話に添える", "", "")}</div><div class="row wrap">${btn("小さい", { v: "sm" })}${btn("小さい", { v: "soft sm", icon: "plus" })}</div></section>
  <section class="card pad stack"><h2>入力</h2><div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:16px"><label class="field"><span class="label">名前</span><input class="input" value="ワークチェア"><span class="help">一覧に出る名前</span></label><label class="field"><span class="label">期日</span><input class="input" value="13月8日" aria-invalid="true"><span class="err">${i("alert-triangle")}月は 1 から 12 で書く</span></label><label class="field"><span class="label">備考</span><textarea class="input" placeholder="品番や買った店など"></textarea></label><div class="stack"><label class="check"><input type="checkbox" checked>手放した物も</label><label class="check"><input type="radio" name="r" checked>毎晩</label><label class="row"><input type="checkbox" class="switch" checked aria-label="動かす">動かす</label>${select("担当", ["あなた"])}</div></div></section>
  <section class="card pad stack"><h2>状態のバッジ</h2><div class="row wrap">${Object.keys(ST).map(state).join("")}</div><div class="row wrap">${Object.values(SRC).join("")}${badge(AIDE, "aide")}${badge("つながっている", "ok", "check")}${badge("優先度 高", "danger", "flag")}<span class="count warn">2</span></div><h2>チップ</h2><div class="row wrap"><button type="button" class="chip" aria-pressed="true">すべて 8</button><button type="button" class="chip" aria-pressed="false">家具 2</button><a class="chip" href="#parts">${i("user")}松本 由紀</a><span class="chip">${i("file-text")}見積もり.pdf<button class="x" aria-label="見積もり.pdf を外す">${i("x")}</button></span></div></section>
  <section class="card pad stack"><h2>タブと切り替え</h2><div style="overflow-x:auto"><div class="tabs" style="border-radius:var(--r);width:max-content;max-width:100%"><a class="tab" href="#parts" aria-current="page">${i("home")}ホーム</a><a class="tab" href="#parts">${i("calendar")}カレンダー</a></div></div><div class="segs" role="group" aria-label="見え方の見本" style="align-self:flex-start"><button type="button" class="seg" aria-pressed="false" data-act="press">日</button><button type="button" class="seg" aria-pressed="true" data-act="press">週</button><button type="button" class="seg" aria-pressed="false" data-act="press">月</button></div></section>
  <section class="card pad stack"><h2>アバター</h2><div class="row wrap">${PEOPLE.map((p) => avatar(p)).join("")}${avatar(ORGS[0])}${face(36)}${avatar(PEOPLE[0], "xl")}</div></section>
  <section class="card pad stack"><h2>知らせ</h2><div class="callout">${i("info-circle")}ほかのワークグループの 3 件は出ていない</div><div class="callout ok">${i("circle-check")}つなぎ直した</div><div class="callout warn">${i("alert-triangle")}古くなったかもしれない</div><div class="callout danger">${i("alert-triangle")}読めなかった。つながりを確かめて、読み直す</div><div class="row wrap">${btn("トーストを出す", { act: "toast" })}${btn("モーダルを開く", { act: "remove" })}${btn("板を開く", { act: "status" })}</div></section>
  <section class="card pad stack"><h2>スケルトンと、空のとき</h2><div class="list"><div class="item mid"><div class="sk" style="width:36px;height:36px;border-radius:50%"></div><div class="main"><div class="sk" style="height:14px;width:55%"></div><div class="sk" style="height:11px;width:35%"></div></div></div></div><div class="list">${empty(`今日の予定はない ${btn("予定を足す", { v: "sm", act: "todo" })}`)}</div><div class="meter"><i style="width:40%"></i></div></section></div>`;
}

/* ---------- the look ---------- */
const store = { get: (k) => { try { return localStorage.getItem(`famulen-${k}`); } catch { return null; } }, set: (k, v) => { try { localStorage.setItem(`famulen-${k}`, v); } catch { /* kept for this page only */ } } };
const DEFAULT = { accent: "green", density: "normal", radius: "medium", heading: "gothic", conv: "drill", dest: "seg" };
const themeNow = () => root.dataset.theme ?? "system";
const themeSegs = (words) => [["system", "システム", "device-desktop"], ["light", "ライト", "sun"], ["dark", "ダーク", "moon"]].map(([k, t, ic]) => `<button type="button" role="radio" class="seg ${words ? "" : "only"}" aria-checked="${themeNow() === k}" tabindex="${themeNow() === k ? 0 : -1}" ${words ? "" : `aria-label="${t}" title="${t}"`} data-theme-pick="${k}">${i(ic)}${words ? t : ""}</button>`).join("");
const choice = (name, opts) => `<div class="segs" role="radiogroup" aria-label="${name}" style="align-self:flex-start">${opts.map(([v, t]) => `<button type="button" role="radio" class="seg" aria-checked="${(root.dataset[name] ?? DEFAULT[name]) === v}" tabindex="${(root.dataset[name] ?? DEFAULT[name]) === v ? 0 : -1}" data-choice="${name}:${v}">${t}</button>`).join("")}</div>`;
function applyTheme(t) {
  if (t === "system") delete root.dataset.theme; else root.dataset.theme = t;
  const c = getComputedStyle(document.body).backgroundColor;
  let m = $('meta[name="theme-color"]');
  if (!m) { m = document.createElement("meta"); m.name = "theme-color"; document.head.append(m); }
  m.content = c;
}
const tryPanel = () => `<div class="pop stack" id="try" style="top:52px;right:16px" hidden><div class="row"><b class="grow">PoC の見え方を試す</b>${ib("x", "閉じる", "try")}</div><span class="sm muted">テーマ</span><div class="segs" role="radiogroup" aria-label="テーマ">${themeSegs(true)}</div><span class="sm muted">アクセントの色</span>${choice("accent", [["green", "深緑"], ["indigo", "夜藍"], ["plum", "葡萄"]])}<span class="sm muted">密度</span>${choice("density", [["compact", "詰める"], ["normal", "ふつう"], ["roomy", "ゆったり"]])}<span class="sm muted">角丸</span>${choice("radius", [["small", "小"], ["medium", "中"], ["large", "大"]])}<span class="sm muted">見出しの書体</span>${choice("heading", [["gothic", "ゴシック"], ["mincho", "明朝"]])}<div class="row wrap"><a class="btn sm" href="#approvals">確認の画面</a><a class="btn sm" href="#threads">話の見せ方の案</a><a class="btn sm" href="#parts">部品の見本帳</a>${btn(root.dataset.frame ? "広い画面で見る" : "スマホ幅で見る", { v: "sm", icon: "device-mobile", act: "frame" })}</div></div>`;

/* ---------- drawing ---------- */
const route = () => { const h = location.hash.slice(1); return h === "chat" || P[h] ? h || "home" : "home"; };
let chatOpen = false;
const ticked = new Set();
const typed = new Map();
/** What a field is, wherever it is drawn again. */
function fieldKey(el) {
  const card = el.closest("[data-ask]");
  if (card) return `${card.dataset.ask}:${el.type === "radio" ? $$("input[type=radio]", card).indexOf(el) : "text"}`;
  return `${route()}:${el.getAttribute("aria-label") ?? el.closest("label")?.textContent.trim() ?? el.placeholder}`;
}
const toggles = (el) => el.type === "radio" || el.type === "checkbox";
function restore() {
  for (const el of $$("#pane :is(input, select, textarea), #conv input")) {
    const v = typed.get(fieldKey(el));
    if (v !== undefined) { if (toggles(el)) el.checked = v; else el.value = v; }
    if (el.matches("[data-search]") && el.value) showMatches(el);
  }
}
function showMatches(el) {
  const q = el.value.trim();
  $$(".item, .thing", el.closest(".page")).forEach((x) => (x.hidden = !!q && !x.textContent.includes(q)));
}
let attached = [];
function shell() {
  $("#app").innerHTML = `<div class="shell" id="shell">
  <section class="chat" id="chat" aria-label="${AIDE}との会話">
    <div class="conv" id="conv"></div><div id="arrival"></div>
    <button type="button" class="peek m-only" data-act="unfold" aria-label="会話を開く" id="peek"></button>
    <form class="composer" id="composer"><div class="destbox" id="dest"></div><div class="ctx" id="ctx"></div><label class="sr" for="say">${AIDE}に頼む</label><textarea id="say" rows="1" placeholder="${AIDE}に頼む"></textarea><div class="row tools"><button type="button" class="btn icon ghost sm clip" aria-label="ファイルを添える" title="ファイルを添える" data-act="file">${i("paperclip")}</button><span class="gap"></span><span class="xs muted kbd">⌘ Enter で送る</span><button type="submit" class="btn primary sm send" aria-label="送る">${i("send-2")}<span>送る</span></button></div></form>
    <div class="edge" role="separator" aria-orientation="vertical" aria-label="会話の幅" tabindex="0" id="edge"></div></section>
  <div class="main"><header class="m-head m-only" id="mhead"></header>
    <header class="bar"><div class="name">${face(32)}<div><b>${AIDE}</b><small>Famulen</small></div></div><div class="wgs" role="group" aria-label="ワークグループ" id="wgs"></div><span class="gap"></span><button type="button" class="alive" data-act="status" aria-label="秘書の状態"><span class="dot"></span>考えている<span class="count warn" title="あなたを待っているもの" data-waiting=""></span>${i("chevron-down").replace('class="i"', 'class="i" style="width:14px"')}</button><div class="segs" role="radiogroup" aria-label="テーマ" id="theme"></div>${ib("adjustments-horizontal", "PoC の見え方を試す", "try", "ghost")}</header>
    <nav class="tabs" id="tabs" aria-label="セクション"></nav><main class="pane" id="pane"></main></div>
  <nav class="tabbar m-only" id="tabbar" aria-label="セクション"></nav></div><div id="trybox"></div><div id="toast" role="status" hidden></div>
  <dialog class="modal" id="modal" aria-labelledby="modal-t"><div class="stack"><h2 id="modal-t">この項目を消す</h2><p class="sm muted">消すと手では戻せない。つながっている記録からも外れる。</p><div class="row" style="justify-content:flex-end">${btn("やめておく", { act: "shut" })}${btn("消す", { v: "danger-solid", icon: "trash", act: "shut" })}</div></div></dialog>
  <dialog class="modal" id="image" aria-label="添えた画像"><div class="stack"><div class="row"><b class="grow">座面の破れ</b>${ib("x", "閉じる", "shut")}</div><div id="imagebox"></div></div></dialog>
  <dialog class="modal" id="wgsheet" aria-labelledby="wg-t"><div class="stack"><h2 id="wg-t">ワークグループ</h2><p class="sm muted">会話と画面の両方が絞られる</p><div class="list" id="wglist"></div>${btn("ワークグループを足す", { v: "ghost", icon: "plus", act: "todo" })}</div></dialog>
  <dialog class="side" id="status" aria-labelledby="st-t"><div class="dbar"><h2 class="grow" id="st-t">秘書の状態</h2>${ib("x", "閉じる", "shut")}</div><div class="dscroll" style="max-height:calc(100% - 48px)"><dl class="kv"><dt>していること</dt><dd>仕事をしている（四半期報告の突き合わせ）</dd><dt>待っている入力</dt><dd>1 件</dd><dt>あなたを待っているもの</dt><dd><span data-waiting=""></span> 件</dd><dt>定期の仕事</dt><dd>2 件</dd></dl><section class="sec"><h3>あなたを待っているもの</h3><ul class="list" id="asks"></ul></section><section class="sec"><h3>これからの予定</h3><ul class="list"><li class="item mid"><div class="main"><span class="t">起床予約、15:00</span><div class="sub">採用の打ち合わせの宿題を確かめる</div></div>${ib("x", "取り消す", "todo")}</li><li class="item mid"><div class="main"><span class="t">夢、明日 4:00</span></div></li></ul></section><section class="sec"><div class="row"><h3 class="grow">最近の入力</h3>${btn("JSONL で書き出す", { v: "ghost sm", act: "todo" })}</div><ul class="list">${[["コンソール", "松本さんに返事を書いて", "済", "ok"], ["Slack", "椅子の座面が破れた", "仕事中", "info"], ["見張り", "家のメール 3 通", "待ち", ""]].map(([a, b, c, t]) => `<li class="item mid"><div class="main"><span class="t">${b}</span><div class="sub">${a}</div></div>${badge(c, t)}</li>`).join("")}</ul></section></div></dialog>`;
}
let first = true;
let listTop = 0;
/** `keep` the page where it was read to; the conversation stays put unless it is to go to its `end`. */
function draw(keep, end) {
  const r = route();
  const page = r === "chat" ? "home" : r;
  const top = keep ? $(".page", $("#pane"))?.scrollTop : 0;
  const key = () => `${conv.view}:${conv.view === "thread" ? conv.id : ""}:${convKind()}`;
  const scroll = $("#conv .talk")?.scrollTop ?? 0;
  if ($("#conv .tlist")?.clientHeight) listTop = $("#conv .tlist").scrollTop;
  const same = $("#conv").dataset.key === key();
  root.dataset.wg = wg;
  const at = TAB_OF[page] ?? page;
  const tab = ([k, t, ic]) => `<a class="tab" href="#${k}" ${k === at ? 'aria-current="page"' : ""}>${i(ic)}${t}</a>`;
  $("#tabs").innerHTML = TABS.map(tab).join("") + '<span class="gap"></span>' + ASIDE.map(tab).join("");
  const bottom = ["home", "calendar", "tasks", "memos"];
  $("#tabbar").innerHTML = TABS.filter(([k]) => bottom.includes(k)).map(([k, t, ic]) => `<a href="#${k}" ${k === at ? 'aria-current="page"' : ""}>${i(ic)}<span>${t}</span></a>`).join("") + `<a href="#more" ${bottom.includes(at) ? "" : 'aria-current="page"'}>${i("layout-grid")}<span>ほか</span></a>`;
  $("#wgs").innerHTML = Object.entries(WG).map(([k, t]) => `<button type="button" aria-pressed="${k === wg}" data-wg-pick="${k}"><span class="dot ${k}"></span>${t}</button>`).join("") + `${ib("plus", "ワークグループを足す", "todo")}<span class="wg-hint">${wg === "all" ? "" : `${WG[wg]}のものだけを使う`}</span>`;
  $("#wglist").innerHTML = Object.entries(WG).map(([k, t]) => `<div class="item mid"><span class="dot ${k}"></span><div class="main"><button type="button" class="pick t" data-wg-pick="${k}">${t}</button></div>${k === wg ? i("check") : ""}</div>`).join("");
  $("#mhead").innerHTML = `${bottom.includes(at) || page === "more" ? "" : `<a class="btn icon ghost back" href="#more" aria-label="ほかへ戻る">${i("chevron-left")}</a>`}<h1>${page === "memory" && sel.layer ? (sel.layer === "dream" ? "夢" : MEM[sel.layer].t) : TITLE[page]}</h1>${ib("adjustments-horizontal", "PoC の見え方を試す", "try", "ghost")}<button type="button" class="m-wg" data-act="wgsheet" aria-label="ワークグループを選ぶ（いまは${WG[wg]}）"><span class="dot"></span>${WG[wg]}${i("chevron-down").replace('class="i"', 'class="i" style="width:14px"')}</button>`;
  for (const el of $$("[data-waiting]")) { el.textContent = el.dataset.waiting + waiting().length; if (el.matches(".count")) el.hidden = !waiting().length; }
  $("#asks").innerHTML = waiting().map((k) => `<li class="item mid"><div class="main"><a class="pick t" href="#approvals" data-act="shut">${ASKS[k]}</a></div>${k === "cut" ? badge("止まった") : badge("確かめる", "warn", "hand-stop")}</li>`).join("") || empty("なし");
  $("#theme").innerHTML = themeSegs(false);
  $("#pane").innerHTML = P[page]();
  if (page === "memory" && sel.layer) { const b = $("#mhead .back"); if (b) { b.removeAttribute("href"); b.outerHTML = `<button type="button" class="btn icon ghost back" data-act="layer" data-layer="" aria-label="記憶の一覧へ戻る">${i("chevron-left")}</button>`; } $$("#pane .head .back").forEach((x) => x.remove()); }
  $("#conv").innerHTML = convHtml(convKind());
  $("#conv").dataset.key = key();
  $("#dest").innerHTML = destHtml(destKind());
  $("#arrival").innerHTML = arrivalHtml();
  const now = conv.view === "thread" ? thread(conv.id) : null;
  $("#peek").innerHTML = `${face(24)}<span class="grow cut"><b ${now ? `data-title="${now.id}"` : ""}>${now ? "" : conv.view === "new" ? "新しい話" : "話の一覧"}</b>${now ? ` <span class="muted" data-last="${now.id}"></span>` : ""}</span>${arrived ? badge("結果が届いた", "ok", "check") : ""}<span class="count warn" ${waiting().length ? "" : "hidden"}>${waiting().length ? `待ち ${waiting().length}` : ""}</span>`;
  $("#say").placeholder = now ? "続きを書く" : `${AIDE}に新しく頼む`;
  fillThreads();
  restore();
  const open = !$("#try") || $("#try").hidden;
  $("#trybox").innerHTML = tryPanel();
  $("#try").hidden = open;
  $("#ctx").innerHTML = `<span class="row" style="gap:6px"><span class="dot"></span>${VIEW[page]}を見ている</span>${attached.map((a, n) => `<span class="chip">${i("link")}<span class="cut" style="max-width:180px">${esc(a)}</span><button type="button" class="x" aria-label="${esc(a)}を外す" data-act="detach" data-n="${n}">${i("x")}</button></span>`).join("")}`;
  $("#shell").classList.toggle("chat-open", chatOpen);
  if (keep && $(".page", $("#pane"))) $(".page", $("#pane")).scrollTop = top ?? 0;
  const talk = $("#conv .talk");
  if (talk) {
    const on = $(".tcard.on", talk);
    talk.scrollTop = !same || end || first ? (on ? on.offsetTop - 12 : talk.scrollHeight) : scroll;
    if (talk.clientHeight) first = false;
  }
  if ($("#conv .tlist")) $("#conv .tlist").scrollTop = listTop;
  document.title = `${TITLE[page]}、Famulen コンソールの PoC`;
}
let toastTimer;
function toast(t) {
  const el = $("#toast");
  el.innerHTML = `${i("circle-check")}<span>${esc(t)}</span>`;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), 2600);
}
/** Redraw around the control that was used, which the redraw replaces. */
function redraw() {
  const el = document.activeElement;
  const same = el && el !== document.body && ["data-sel", "data-entry", "data-open", "data-what", "data-job", "data-k", "data-go", "data-key", "aria-label", "data-theme-pick", "data-choice", "data-wg-pick", "data-thread", "data-st", "data-layer", "data-v", "data-act"].flatMap((a) => (el.hasAttribute(a) ? [`[${a}="${CSS.escape(el.getAttribute(a))}"]`] : [])).join("");
  const box = same && ["#try", "#theme", "#wgs", "#pane", "#conv"].find((b) => el.closest(b));
  draw(true);
  if (box) (($(`${box} ${same}`) ?? $("#pane .detail .btn"))?.focus({ preventScroll: true }));
}
const go = (h) => { chatOpen = false; if (location.hash === `#${h}`) redraw(); else location.hash = h; };

document.addEventListener("click", (e) => {
  const t = e.target.closest("[data-act],[data-sel],[data-entry],[data-open],[data-theme-pick],[data-choice],[data-wg-pick],[data-thread]");
  if (!t) { if (e.target.closest("#tabbar a") && chatOpen) { chatOpen = false; draw(true); } return; }
  const d = t.dataset;
  if (d.themePick) { store.set("theme", d.themePick); applyTheme(d.themePick); return redraw(); }
  if (d.choice) { const [k, v] = d.choice.split(":"); root.dataset[k] = v; store.set(k, v); if (k === "conv") $("#shell").style.gridTemplateColumns = ""; return redraw(); }
  if (d.wgPick) { wg = d.wgPick; $("#wgsheet").close(); for (const k of ["tasks", "delegated", "parties", "things", "contracts", "entry", "st", "thingscat", "contractscat"]) delete sel[k]; if (conv.view === "thread" && !inWg(thread(conv.id))) return goConv("list"); return redraw(); }
  if (d.open) { e.preventDefault(); const [k, id] = d.open.split(":"); if (k === "memory") { sel.layer = Object.keys(ENTRIES).find((x) => ENTRIES[x].some((m) => m.id === id)); sel.entry = id; } else sel[k] = id; chatOpen = false; return go(k); }
  if (d.sel) { const at = d.sel.indexOf(":"); sel[d.sel.slice(0, at)] = d.sel.slice(at + 1); return redraw(); }
  if (d.thread) return goConv("thread", d.thread);
  if (d.entry) { sel.entry = d.entry; return redraw(); }
  const act = {
    todo: () => { e.preventDefault(); toast("PoC なので、この先は作っていない"); },
    toast: () => toast("保存した"),
    go: () => go(d.go),
    opentalk: () => { chatOpen = true; goConv("thread", d.id); },
    openjob: () => { sel.delegated = d.job; chatOpen = false; go("delegated"); },
    answerjob: () => { sel.delegated = d.job; redraw(); },
    check: () => { const on = t.getAttribute("aria-pressed") !== "true"; t.setAttribute("aria-pressed", on); ticked[on ? "add" : "delete"](d.what); toast(on ? "済にした" : "戻した"); },
    press: () => $$("[data-act=press]", t.parentElement).forEach((b) => b.setAttribute("aria-pressed", b === t)),
    st: () => { sel.st = d.st; redraw(); },
    hv: () => { sel[`${d.k}v`] = d.v; redraw(); },
    cat: () => { sel[`${d.k}cat`] = d.v; redraw(); },
    layer: () => { sel.layer = d.layer || undefined; delete sel.entry; draw(); },
    close: () => { delete sel[d.key]; draw(true); },
    attach: () => { if (!attached.includes(d.what)) attached.push(d.what); redraw(); toast("会話に添えた"); },
    file: () => { attached.push("見積もり.pdf"); redraw(); },
    detach: () => { attached.splice(Number(d.n), 1); draw(true); $("#say").focus(); },
    answer: () => { const card = t.closest("[data-ask]"); const k = card.dataset.ask; const said = d.a === "reply" ? `「${$("input.input", card).value.trim() || $("input:checked + span b", card).textContent.replace("（おすすめ）", "")}」と答えた` : d.a; const at = t.closest("#pane") ? "#pane" : "#conv"; answered[k] = said; draw(true); $(`${at} [data-done="${k}"]`)?.focus({ preventScroll: true }); toast(said); },
    remove: () => $("#modal").showModal(),
    status: () => $("#status").showModal(),
    wgsheet: () => $("#wgsheet").showModal(),
    image: () => { $("#imagebox").innerHTML = t.innerHTML; $("#image").showModal(); },
    shut: () => t.closest("dialog").close(),
    try: () => { $("#try").hidden = !$("#try").hidden; },
    frame: () => { if (root.dataset.frame) delete root.dataset.frame; else root.dataset.frame = "phone"; $("#shell").style.gridTemplateColumns = ""; chatOpen = false; draw(true); },
    unfold: () => { chatOpen = true; first = true; draw(true, true); $("#say").focus({ preventScroll: true }); },
    fold: () => { chatOpen = false; draw(true); },
  }[d.act];
  (act ?? (THREAD_ACTS[d.act] && (() => THREAD_ACTS[d.act](d, t))))?.();
});
for (const id of ["modal", "status", "wgsheet", "image"]) document.addEventListener("click", (e) => { if (e.target.id !== id) return; const r = e.target.getBoundingClientRect(); if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) e.target.close(); });
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && $("#try") && !$("#try").hidden) $("#try").hidden = true;
  if (e.target.id === "say" && e.key === "Enter" && (e.metaKey || e.ctrlKey)) $("#composer").requestSubmit();
  if (e.target.id === "edge" && ["ArrowLeft", "ArrowRight"].includes(e.key)) setChat($("#chat").offsetWidth + (e.key === "ArrowLeft" ? -24 : 24));
  const radios = e.target.closest?.('[role="radiogroup"]');
  if (radios && e.target.matches('[role="radio"]') && ["ArrowLeft", "ArrowRight"].includes(e.key)) { const rs = $$('[role="radio"]', radios); const next = rs[(rs.indexOf(e.target) + (e.key === "ArrowRight" ? 1 : rs.length - 1)) % rs.length]; next.focus(); next.click(); }
});
/** Whether the phone's layout is drawn, as on a phone or in its frame. */
const narrow = () => $("#app").clientWidth <= 700;
const setChat = (w) => { const all = $("#app").clientWidth; const least = convKind() === "side" && all > 1160 ? 560 : 320; $("#shell").style.gridTemplateColumns = narrow() ? "" : `${Math.max(least, Math.min(all - 420, 860, w))}px minmax(0, 1fr)`; };
document.addEventListener("pointerdown", (e) => { if (e.target.id !== "edge") return; e.target.setPointerCapture(e.pointerId); const move = (m) => setChat(m.clientX); e.target.addEventListener("pointermove", move); e.target.addEventListener("pointerup", () => e.target.removeEventListener("pointermove", move), { once: true }); });
document.addEventListener("submit", (e) => { e.preventDefault(); const v = $("#say").value.trim(); if (!v) return; $("#say").value = ""; attached = []; sendSaid(v); delete drafts.new; delete drafts[conv.id]; draw(true, true); });
document.addEventListener("input", (e) => {
  const el = e.target;
  if (!el.closest("#pane, #conv")) return;
  const others = el.type === "radio" ? $$(`input[name="${el.name}"]`) : [el];
  for (const o of others) typed.set(fieldKey(o), toggles(o) ? o.checked : o.value);
  if (el.matches("[data-search]")) showMatches(el);
});
addEventListener("hashchange", () => { chatOpen = route() === "chat"; draw(false, chatOpen); });
addEventListener("resize", () => { if (narrow()) $("#shell").style.gridTemplateColumns = ""; });
addEventListener("storage", (e) => { if (e.key === "famulen-theme") { applyTheme(e.newValue ?? "system"); draw(true); } });
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => applyTheme(themeNow()));
for (const k of Object.keys(DEFAULT)) { const v = store.get(k); if (v) root.dataset[k] = v; }
shell();
chatOpen = route() === "chat";
draw();
applyTheme(themeNow());
