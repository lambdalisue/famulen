// Made-up records, as real as the console's own would read.
const AIDE = "ハル";
const ST = {
  running: ["動いている", "info", "refresh", "run"],
  queued: ["順番待ち", "", "clock"],
  blocked: ["先の仕事を待つ", "dash", "lock"],
  ask: ["あなたの答え待ち", "warn", "hand-stop"],
  done: ["済んだ", "ok", "check"],
  failed: ["うまくいかなかった", "danger", "alert-triangle"],
};
const JOBS = [
  { id: "j1", st: "ask", t: "引っ越し業者 3 社の見積もりを比べ、候補を 1 社に絞る", wg: "home", to: "手代", why: "荷物の量を答えると続きを始める", when: "12 分待っている", due: "11月28日" },
  { id: "j2", st: "ask", t: "経費精算の CSV 取り込みを、新しい書式に合わせて直す", wg: "work", to: "Adroit", why: "始めてよいかの承認を待っている", when: "2 時間待っている", due: "11月30日", pri: 1 },
  { id: "j3", st: "running", t: "四半期報告の数字を、基幹の集計と突き合わせる", wg: "work", to: "Adroit", pct: 66, step: "食い違いの洗い出し", when: "38 分", due: "今日", pri: 1 },
  { id: "j4", st: "running", t: "社内勉強会の録画から、議事と宿題を書き起こす", wg: "work", to: "手代", pct: 20, step: "録画を読んでいる", when: "6 分" },
  { id: "j5", st: "queued", t: "年賀状の宛先を、相手の記録と照らして直す", wg: "home", to: "手代", step: "次の次に始まる", when: "今日 9:12 に委ねた", due: "12月10日" },
  { id: "j6", st: "blocked", t: "四半期報告の下書きを作る", wg: "work", to: "Adroit", step: "「四半期報告の数字を、基幹の集計と突き合わせる」が済むまで", when: "昨日委ねた", due: "11月27日" },
  { id: "j7", st: "done", t: "ふるさと納税の控除上限を、源泉徴収票から試算する", wg: "home", to: "手代", step: "14 分で済んだ。結果は経過に", when: "今日 8:40" },
  { id: "j8", st: "failed", t: "古い写真を年ごとのアルバムに振り分ける", wg: "home", to: "手代", step: "写真の置き場所に届かなかった", when: "昨日 23:10" },
];
const TASKS = [
  ["今日", [
    { t: "松本さんに設計レビューの返事をする", wg: "work", who: "あなた", due: "今日 17:00", pri: 1, note: "返事の下書きあり" },
    { t: "四半期報告の数字を、基幹の集計と突き合わせる", wg: "work", who: "委託", due: "今日", st: "running", job: "j3" },
    { t: "保育園の連絡帳に、金曜の迎えを書く", wg: "home", who: "あなた", due: "今日" },
  ]],
  ["今週", [
    { t: "四半期報告の下書きを作る", wg: "work", who: "委託", due: "明日 11月27日", st: "blocked", job: "j6" },
    { t: "引っ越し業者 3 社の見積もりを比べ、候補を 1 社に絞る", wg: "home", who: "委託", due: "11月28日", st: "ask", job: "j1" },
    { t: "椅子の座面の交換部品を注文する", wg: "home", who: AIDE, due: "11月29日" },
  ]],
  ["いつか", [
    { t: "書斎の照明を調光できるものに替える", wg: "home", who: "あなた", due: "", labels: ["住まい"] },
    { t: "技術書典の原稿の目次を考える", wg: "work", who: "あなた", due: "", labels: ["執筆"] },
    { t: "ルーターのファームウェアを上げる", wg: "home", who: AIDE, due: "" },
  ]],
];
const ROUTINES = [
  { t: "朝の支度：今日の予定と、夜のうちに届いたものをまとめる", when: "毎日 7:30", next: "明日 7:30", on: 1 },
  { t: "週報の下書き", when: "毎週金曜 16:00", next: "明日 16:00", on: 1 },
  { t: "家計簿の取り込み", when: "毎月 1 日", next: "12月1日", on: 0 },
];
const EVENTS = [
  { s: "9:30", e: "10:00", t: "朝会", sub: "オンライン、6 人", wg: "work", past: 1 },
  { s: "11:00", e: "12:00", t: "設計レビュー", sub: "会議室 3A、4 人", wg: "work", now: 1, meet: 1 },
  { s: "14:00", e: "14:30", t: "採用の打ち合わせ", sub: "オンライン、3 人", wg: "work", meet: 1 },
  { s: "18:00", e: "18:30", t: "保育園の迎え", sub: "ひだまり保育園", wg: "home" },
];
const PEOPLE = [
  { id: "p1", n: "松本 由紀", i: "松", h: 168, role: "上司", sub: "設計部", org: "青葉システム", wg: "work", alias: "まつもとさん", seen: "今日" },
  { id: "p2", n: "田所 健", i: "田", h: 216, role: "同僚", sub: "基幹チーム", org: "青葉システム", wg: "work", alias: "tadoken", seen: "昨日" },
  { id: "p3", n: "グエン・リン", i: "リ", h: 28, role: "部下", sub: "基幹チーム", org: "青葉システム", wg: "work", alias: "linh", seen: "11月24日" },
  { id: "p4", n: "森川 千秋", i: "森", h: 322, role: "友人", sub: "写真仲間", org: "", wg: "home", alias: "ちあき", seen: "11月22日" },
  { id: "p5", n: "大野 朝子", i: "大", h: 268, role: "担任", sub: "ひだまり保育園", org: "ひだまり保育園", wg: "home", alias: "朝子先生", seen: "11月25日", guess: 1 },
];
const ORGS = [
  { id: "o1", n: "青葉システム", i: "青", h: 198, role: "勤め先", sub: "3 人", wg: "work", org: 1, seen: "今日" },
  { id: "o2", n: "ひだまり保育園", i: "ひ", h: 42, role: "子の園", sub: "1 人", wg: "home", org: 1, seen: "11月25日" },
];
const MEM = {
  pending: { t: "保留中の判断", icon: "hourglass", c: "3", d: "急がない。根拠を待っている", note: "まったく急がない。答えなくてよい。夢のたびに根拠を数え直し、そろえばハルが自分で決める。" },
  episodic: { t: "気づき", icon: "bulb", c: "3", d: "雑に書く受信箱。夢が整理したら消える", note: "ここにあるのは夢を待つものだけで、夢が整理したら消える。" },
  working: { t: "作業記憶", icon: "activity", c: "1", d: "いま進めている仕事の途中経過", note: "済めば消える。" },
  core: { t: "心得", icon: "quote", c: "4 / 150 行", d: "毎回の会話で必ず読む", note: "小さく保つ。上限を超えそうになると、夢の中で使われていないものから外す。" },
  long: { t: "長期記憶", icon: "brain", c: "3", d: "意味記憶・手続き記憶・展望記憶", note: "覚えたこと。必要なときに読みに行く。" },
  forgotten: { t: "忘却", icon: "archive", c: "1", d: "古くなった記憶。戻せる", note: "ふだんは読まないが、消してはいないので戻せる。" },
};
const ENTRIES = {
  pending: [
    { id: "m1", t: "森川さんの誕生日は 12 月の前半らしい", sub: "根拠 1 / 要る 2", src: "guess", wg: "home", ev: [1, 2] },
    { id: "m2", t: "会議は午前より午後のほうがよい、に変わったかもしれない", sub: "根拠 2 / 要る 3", src: "guess", against: 1, wg: "work", ev: [2, 3] },
    { id: "m3", t: "田所さんが年明けに異動するらしい", sub: "根拠 1 / 要る 2", src: "guess", wg: "work", ev: [1, 2] },
  ],
  episodic: [
    { id: "m4", g: "11月26日", time: "10:42", t: "設計レビューの前日までに、論点を 3 つに絞って渡してほしい", src: "said", wg: "work" },
    { id: "m5", g: "11月26日", time: "09:15", t: "大野先生は、金曜の迎えを 18 時に変えたことを園に伝えてほしいと書いている", src: "guess", wg: "home" },
    { id: "m6", g: "11月25日", time: "21:08", t: "椅子の座面がへたってきたので、交換部品を調べた", src: "confirmed", wg: "home" },
  ],
  working: [{ id: "m7", t: "四半期報告：突き合わせのあと、下書きに進む", sub: "昨日から", src: "confirmed", wg: "work" }],
  core: [
    { id: "m8", t: "返事は日本語で、結論から書く", src: "said", wg: "", sub: "使った 318 回" },
    { id: "m9", t: "18 時から 20 時までは、急ぎでなければ知らせない", src: "said", wg: "home", sub: "使った 96 回" },
    { id: "m10", t: "社外へのメールは、送る前に必ず文面を見せる", src: "said", wg: "work", sub: "使った 57 回" },
    { id: "m11", t: "会議と会議のあいだは 15 分空ける", src: "confirmed", wg: "work", sub: "使った 41 回" },
  ],
  long: [
    { id: "m12", g: "意味記憶：人", t: "松本さんは、資料は PDF より共有ドキュメントを好む", src: "confirmed", wg: "work", sub: "使った 12 回" },
    { id: "m13", g: "手続き記憶：経費", t: "出張の精算は、領収書をまとめてから月末に一度で出す", src: "said", wg: "work", sub: "使った 6 回" },
    { id: "m14", g: "展望記憶", t: "12 月の第 1 週に、年賀状の枚数を尋ねる", src: "said", wg: "home", sub: "12月1日から" },
  ],
  forgotten: [{ id: "m15", g: "2026-11", t: "週報は木曜の夕方に書く", src: "said", wg: "work", sub: "金曜に変わったので置き換えた" }],
};
// Drawn for this page: plain outlines on a 64 grid.
const PIC = {
  laptop: '<path d="M14 16h36v24H14zM8 46h48l-3 4H11z"/><path d="M26 46h12"/>',
  chair: '<path d="M22 8h20l-2 22H24zM18 34h28v6H18zM32 40v10M20 56l12-6 12 6M16 26v10M48 26v10"/>',
  phones: '<path d="M14 40V32a18 18 0 0 1 36 0v8"/><rect x="10" y="36" width="10" height="16" rx="4"/><rect x="44" y="36" width="10" height="16" rx="4"/>',
  camera: '<path d="M8 20h12l4-6h16l4 6h12v30H8z"/><circle cx="32" cy="34" r="10"/><circle cx="32" cy="34" r="4"/>',
  keyboard: '<rect x="6" y="20" width="52" height="24" rx="3"/><path d="M13 27h2M21 27h2M29 27h2M37 27h2M45 27h6M13 32h4M23 32h2M31 32h2M39 32h2M47 32h4M18 38h28"/>',
  book: '<path d="M14 8h32v44H18a4 4 0 0 0-4 4z"/><path d="M14 56h32M22 18h16M22 24h10"/>',
  router: '<rect x="8" y="34" width="48" height="16" rx="3"/><path d="M16 34V14M48 34V14M16 42h2M24 42h2M40 42h8M24 20a11 11 0 0 1 16 0M28 25a6 6 0 0 1 8 0"/>',
  lamp: '<path d="M26 10h12l8 18H18zM32 28v24M20 54h24"/>',
};
const THINGS = [
  { id: "t1", t: "14 インチ ノート PC（仕事用）", cat: "PC", pic: "laptop", wg: "work", sub: "2026年3月、会社から貸与", place: "書斎", price: "", st: "" },
  { id: "t2", t: "ワークチェア", cat: "家具", pic: "chair", wg: "home", sub: "2022年6月、家具店", place: "書斎", price: "¥128,000", st: "", due: "保証 あと 12 日" },
  { id: "t3", t: "ノイズキャンセリング ヘッドホン", cat: "家電", pic: "phones", wg: "home", sub: "2025年11月", place: "書斎", price: "¥49,500", st: "" },
  { id: "t4", t: "ミラーレス カメラ", cat: "家電", pic: "camera", wg: "home", sub: "注文番号 2026-0915", place: "", price: "¥238,000", st: "注文中", due: "届く予定 11月30日" },
  { id: "t5", t: "分割キーボード", cat: "PC", pic: "keyboard", wg: "work", sub: "2024年1月", place: "会社", price: "¥42,800", st: "" },
  { id: "t6", t: "日本語組版の考え方", cat: "本", pic: "book", wg: "home", sub: "2026年8月", place: "本棚 2 段目", price: "¥3,960", st: "" },
  { id: "t7", t: "Wi-Fi ルーター", cat: "家電", pic: "router", wg: "home", sub: "2023年2月", place: "廊下", price: "¥24,000", st: "", old: 1 },
  { id: "t8", t: "読書灯", cat: "家具", pic: "lamp", wg: "home", sub: "2021年10月", place: "寝室", price: "¥9,800", st: "" },
];
const CONTRACTS = [
  { id: "c1", t: "光回線", cat: "回線", sub: "北星ネット、プラン 1G", price: "¥5,280 / 月", wg: "home", due: "更新月 12月", warn: 1, h: 205, i: "光" },
  { id: "c2", t: "写真編集ソフト", cat: "サブスクリプション", sub: "個人プラン、自動更新", price: "¥1,180 / 月", wg: "home", due: "次の支払い 12月3日", h: 285, i: "写" },
  { id: "c3", t: "火災保険", cat: "保険", sub: "5 年契約", price: "¥18,400 / 年", wg: "home", due: "2028年4月まで", h: 18, i: "保" },
  { id: "c4", t: "統合開発環境", cat: "ライセンス", sub: "会社負担、1 席", price: "¥12,000 / 年", wg: "work", due: "更新 2027年2月", h: 150, i: "開" },
];
const SERVICES = [
  { t: "Google", a: "work@example.com", d: "メールとカレンダーを読んでいる", icon: "brand-google", st: "ok", wg: "work" },
  { t: "Google", a: "home@example.com", d: "メールとカレンダーを読んでいる", icon: "brand-google", st: "warn", msg: "つなぎ直しが要る", wg: "home" },
  { t: "Slack", a: "青葉システム", d: "話しかけと見張り。確認を受けるところ", icon: "brand-slack", st: "ok", wg: "work" },
  { t: "GitHub", a: "example-dev", d: "通知を見張っている", icon: "brand-github", st: "ok", wg: "work" },
  { t: "Adroit", a: "このマシン", d: "委ねた仕事のセッションを動かす", icon: "terminal-2", st: "ok", wg: "" },
  { t: "脆弱性のフィード", a: "https://example.com/feed.xml", d: "30 分ごとに読む", icon: "rss", st: "ok", wg: "work" },
  { t: "Famulen", a: "記録・記憶・予定・返事など", d: "Famulen 自身の道具", icon: "notes", st: "own", wg: "" },
  { t: "秘書の組み込みの道具", a: "ファイル、シェル、ウェブ", d: "モデル CLI が持つ道具", icon: "cpu", st: "own", wg: "" },
];
