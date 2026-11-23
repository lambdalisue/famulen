import { RadioGroup } from "@ark-ui/solid/radio-group";
import { A, createAsync, revalidate } from "@solidjs/router";
import { For, Show, createEffect, createSignal, onCleanup } from "solid-js";
import FolderPicker from "~/components/FolderPicker";
import Icon from "~/components/Icon";
import NotebookCheck from "~/components/NotebookCheck";
import LogSheet from "~/components/logs/LogSheet";
import SecretaryLog from "~/components/logs/SecretaryLog";
import RoleModel from "~/components/RoleModel";
import { Role } from "~/gen/famulen/v1/settings_pb";
import { act, look, say } from "~/lib/act";
import { api } from "~/lib/api";
import { homeQuery, notebookPlaceQuery, personaQuery, slackNamesQuery, writableQuery } from "~/lib/data";
import { coveringPlace, isUnder, wholePath } from "~/lib/places";
import { alertTriangle, externalLink, folder, plus, x } from "~/lib/icons";
import { setView } from "~/lib/view";

/** Ways of speaking the owner picks from; the text is what the secretary is told. */
const TONES = [
  { id: "polite", label: "丁寧で簡潔", text: "丁寧語で話す。結論から先に、短く伝える。" },
  { id: "warm", label: "丁寧であたたかい", text: "丁寧語で話す。用件のあとに、相手を気づかう一言を添える。" },
  { id: "casual", label: "くだけた", text: "友人のように、くだけた言葉で話す。敬語は使わない。" },
  { id: "butler", label: "執事のように", text: "執事のように、格式高く控えめな言葉で話す。" },
  { id: "bullets", label: "箇条書き", text: "前置きを省き、要点だけを箇条書きで伝える。" },
] as const;
const OWN = "own";

const toneOf = (text: string) => TONES.find((t) => t.text === text.trim())?.id ?? (text.trim() ? OWN : "");

const save = (change: Parameters<typeof api.settings.updatePersona>[0]) =>
  act(() => api.settings.updatePersona(change));

function NameSection(props: { name: string }) {
  const slack = createAsync(() => slackNamesQuery());
  const saveName = (el: HTMLInputElement) => {
    const name = el.value.trim();
    if (!name) {
      say("秘書には名前が要る", true);
      el.value = props.name;
      return;
    }
    void save({ name });
  };
  return (
    <section class="pref">
      <div class="pref-head">
        <h2>秘書の名前</h2>
        <p class="sub">Famulen はこの仕組みの名前で、秘書にはあなたが名前を付ける。</p>
      </div>
      <input
        id="p-name"
        class="input pref-name"
        name="name"
        autocomplete="off"
        data-1p-ignore
        data-lpignore="true"
        data-bwignore
        aria-label="秘書の名前"
        value={props.name}
        required
        onChange={(e) => saveName(e.currentTarget)}
      />
      <ul class="pref-uses sub">
        <li>コンソールの見出しと、会話での呼び名</li>
        <li>秘書の自己紹介。秘書を起動するときの前置きで、この名前を名乗るよう伝える</li>
        <li>記録の書き手（「{props.name}が足した」）</li>
      </ul>
      <div class="pref-sub">
        <h3>外のツールでの名前</h3>
        <p class="sub">
          Slack では、アプリの設定にある表示名で出て、メンション名（@ のあと）で呼ばれる。どちらも Slack
          で決まり、ここからは変えられない。
        </p>
        <Show
          when={(slack() ?? []).length > 0}
          fallback={
            <p class="sub">
              Slack のワークスペースを足すと、そこでの名前がここに出る（<A href="/settings/channels">チャネル</A>）。
            </p>
          }
        >
          <ul class="slack-names">
            <For each={slack()}>
              {(n) => (
                <li>
                  <span class="ws">Slack · {n.workspaceName}</span>
                  <Show when={!n.problem} fallback={<span class="sub">名前を読めなかった：{n.problem}</span>}>
                    <span>
                      表示名 <b>{n.displayName}</b>
                    </span>
                    <span class="mono">@{n.handle}</span>
                    <Show when={n.displayName !== props.name}>
                      <span class="pref-warn">
                        <Icon svg={alertTriangle} size={14} />
                        秘書の名前と違う
                      </span>
                    </Show>
                  </Show>
                  <a class="btn small ghost" href={n.settingsUrl} target="_blank" rel="noreferrer">
                    Slack で直す
                    <Icon svg={externalLink} size={14} />
                  </a>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </section>
  );
}

function ToneSection(props: { tone: string; own: string }) {
  // A tone that matches no preset and was never kept as the owner's own is
  // still the owner's own writing.
  const ownText = () => props.own || (toneOf(props.tone) === OWN ? props.tone : "");
  const [chosen, setChosen] = createSignal(toneOf(props.tone));
  createEffect(() => setChosen(toneOf(props.tone)));
  let own: HTMLTextAreaElement | undefined;
  return (
    <section class="pref">
      <div class="pref-head">
        <h2>話し方</h2>
        <p class="sub">秘書があなたに話すときの口調。秘書を起動するときの前置きに入れる。細かい好みは、話すうちに記憶の心得に入る。</p>
      </div>
      <RadioGroup.Root
        class="tones"
        value={chosen()}
        onValueChange={(d) => {
          setChosen(d.value ?? "");
          const t = TONES.find((x) => x.id === d.value);
          if (t) void save({ tone: t.text });
          else if (d.value === OWN) {
            if (ownText()) void save({ tone: ownText() });
            queueMicrotask(() => own?.focus());
          }
        }}
      >
        <RadioGroup.Label class="visually-hidden">話し方</RadioGroup.Label>
        <For each={[...TONES, { id: OWN, label: "自分で書く", text: "" }]}>
          {(t) => (
            <RadioGroup.Item value={t.id} class="tone">
              <RadioGroup.ItemControl class="tone-dot" />
              <RadioGroup.ItemText class="tone-text">
                <b>{t.label}</b>
                <span class="sub">{t.id === OWN ? ownText() || "好きな話し方を文で書く" : t.text}</span>
              </RadioGroup.ItemText>
              <RadioGroup.ItemHiddenInput />
            </RadioGroup.Item>
          )}
        </For>
      </RadioGroup.Root>
      <Show when={chosen() === OWN}>
        <div class="own">
          <textarea
            ref={own}
            id="p-tone"
            class="input"
            name="tone"
            rows="4"
            aria-label="話し方を自分で書く"
            placeholder="丁寧語で、短く。結論から話す。"
            value={ownText()}
            onChange={(e) => {
              const text = e.currentTarget.value.trim();
              void save({ tone: text, ownTone: text });
            }}
          >
            {/* Also the content: the server writes `value` as an attribute, which a textarea ignores, and hydration does not set it. */}
            {ownText()}
          </textarea>
          <span class="sub">この文がそのまま秘書に伝わる。ほかの話し方を選んでも、この文は残る。</span>
        </div>
      </Show>
    </section>
  );
}

/** The whole path a drop names when the browser hands one over: a file URL, or a path written as text. */
function droppedPath(data: DataTransfer): string | undefined {
  const uri = data
    .getData("text/uri-list")
    .split(/\r?\n/)
    .find((l) => l.startsWith("file://"));
  if (uri) return decodeURIComponent(new URL(uri).pathname);
  const text = data.getData("text/plain").trim();
  if (text.startsWith("file://")) return decodeURIComponent(new URL(text).pathname);
  if (/^(\/|~\/|~$)/.test(text)) return text;
  return undefined;
}

function WritableSection() {
  const places = createAsync(() => writableQuery());
  const [path, setPath] = createSignal("");
  const [over, setOver] = createSignal(false);
  const [found, setFound] = createSignal<string[]>([]);
  const fill = (p: string) => {
    setPath(p);
    setFound([]);
  };
  const home = createAsync(() => homeQuery());
  const whole = () => wholePath(path(), home());
  const covering = () => {
    const w = whole();
    return w ? coveringPlace(w, places() ?? []) : undefined;
  };
  const merged = () => {
    const w = whole();
    return w && !covering() ? (places() ?? []).filter((p) => isUnder(p, w)) : [];
  };
  const addAll = async (paths: string[]) =>
    (await act(
      async () => {
        for (const p of paths) await api.settings.addWritablePlace({ path: p });
        return true;
      },
      paths.length > 1 ? `${paths.length} 件足した` : "足した",
    )) === true;
  const add = async (e: SubmitEvent) => {
    e.preventDefault();
    const p = path().trim();
    if (!p || covering()) return;
    if (await addAll([p])) fill("");
  };
  const drop = async (e: DragEvent) => {
    e.preventDefault();
    setOver(false);
    const data = e.dataTransfer;
    if (!data) return;
    const whole = droppedPath(data);
    if (whole) return fill(whole);
    // A folder from the file manager comes as its name alone; find it by name.
    const entry = data.items[0]?.webkitGetAsEntry?.();
    if (!entry) return say("フォルダを落とす", true);
    if (!entry.isDirectory) return say("ファイルではなく、フォルダを落とす", true);
    const result = await look(() => api.settings.findFolders({ name: entry.name }));
    if (!result) return;
    if (result.paths.length === 0) return say(`ホームの下に「${entry.name}」が見つからなかった。選ぶか、書いて足す`, true);
    if (result.paths.length === 1) return fill(result.paths[0]);
    setFound(result.paths);
  };
  return (
    <section class="pref">
      <div class="pref-head">
        <h2>秘書が書ける場所</h2>
        <p class="sub">
          秘書はあなたの Claude Code のスキルを使える。ただしスキルに書かれた道具の許しは受け継がず、道具はいつもの確かめ方で裁かれる。秘書のシェルのコマンドが書けるのは作業の場所と手帳だけなので、スキルがその外に書くときは、その場所をここに足す。読むほうは変わらない。金庫やゲートウェイの置き場所と
          .env は、ここに足しても読み書きできない。変えたことは、秘書が今の仕事を終えてから効く。
        </p>
      </div>
      <Show when={(places() ?? []).length > 0} fallback={<p class="sub">まだない。</p>}>
        <ul class="places" aria-label="秘書が書ける場所">
          <For each={places()}>
            {(p) => (
              <li>
                <span class="mono">{p}</span>
                <button
                  type="button"
                  class="btn small danger"
                  aria-label={`${p} を外す`}
                  onClick={() => void act(() => api.settings.removeWritablePlace({ path: p }), "外した")}
                >
                  <Icon svg={x} size={14} />
                  外す
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <form
        class="place-add"
        classList={{ over: over() }}
        aria-label="フォルダを落とすところ"
        onSubmit={add}
        onDragOver={(e) => {
          e.preventDefault();
          if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
          setOver(true);
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOver(false);
        }}
        onDrop={(e) => void drop(e)}
      >
        <div class="place-row">
          <input
            class="input mono"
            name="path"
            autocomplete="off"
            data-1p-ignore
            data-lpignore="true"
            data-bwignore
            aria-label="書ける場所を足す"
            placeholder="/ か ~/ から書く"
            required
            value={path()}
            onInput={(e) => setPath(e.currentTarget.value)}
          />
          <FolderPicker start={path} places={() => places() ?? []} onAdd={addAll} />
          <button class="btn small primary" disabled={!!covering()}>
            <Icon svg={plus} size={14} />
            足す
          </button>
        </div>
        <Show when={covering()}>
          {(c) => (
            <span class="place-warn" role="status">
              <Icon svg={alertTriangle} size={14} />
              {c() === whole() ? "もう足してある。" : `${c()} の下なので、足さなくても書ける。`}
            </span>
          )}
        </Show>
        <Show when={merged().length > 0}>
          <span class="sub" role="status">
            足すと、下にある {merged().join("、")} はこの場所にまとまる。
          </span>
        </Show>
        <span class="sub">書くか、「選ぶ」で Famulen のマシンのフォルダからいくつでも選ぶか、フォルダをここに落とす。</span>
        <Show when={found().length > 0}>
          <div class="place-found">
            <span class="sub">同じ名前のフォルダがいくつかある。どれか選ぶ。</span>
            <ul class="folder-list" aria-label="見つかったフォルダ">
              <For each={found()}>
                {(p) => (
                  <li>
                    <button type="button" class="folder-item mono" onClick={() => fill(p)}>
                      <Icon svg={folder} size={14} />
                      <span>{p}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </Show>
      </form>
    </section>
  );
}

function NotebookSection() {
  const place = createAsync(() => notebookPlaceQuery());
  const [path, setPath] = createSignal("");
  const moving = () => place()?.movingTo;
  createEffect(() => {
    if (!moving()) return;
    // Famulen starts again on the way, so a call that fails is tried again.
    const timer = setInterval(async () => {
      const now = await api.settings.getNotebookPlace({}).catch(() => undefined);
      if (!now || now.movingTo) return;
      clearInterval(timer);
      await revalidate();
      if (!now.failed) say("手帳を移した");
    }, 1000);
    onCleanup(() => clearInterval(timer));
  });
  const move = async (to: string) => {
    if (await act(() => api.settings.moveNotebook({ path: to }), "")) setPath("");
  };
  return (
    <section class="pref" aria-label="手帳の場所">
      <div class="pref-head">
        <h2>手帳の場所</h2>
        <p class="sub">
          記憶と、手帳に置いた記録（タスク・メモ・予定・相手・物・契約）のある場所。変えると、履歴ごと新しい場所へ移し、前の場所からは手帳のファイルだけを消す。ほかのファイルがあるフォルダも選べる。移すときは、秘書が今の仕事を終えてから
          Famulen が起動し直す。
        </p>
      </div>
      <Show when={place()}>
        {(p) => (
          <>
            <ul class="places" aria-label="今の手帳の場所">
              <li>
                <span class="mono">{p().path}</span>
                <Show when={p().path !== p().defaultPath}>
                  <button
                    type="button"
                    class="btn small"
                    title={p().defaultPath}
                    disabled={!!moving()}
                    onClick={() => void move("")}
                  >
                    既定の場所に戻す
                  </button>
                </Show>
              </li>
            </ul>
            <Show when={moving()}>
              {(to) => (
                <span class="sub" role="status">
                  {to()} へ移すところ。秘書が今の仕事を終えると移る。
                </span>
              )}
            </Show>
            <Show when={p().failed}>
              <span class="place-warn" role="alert">
                <Icon svg={alertTriangle} size={14} />
                移せなかったので、手帳は元の場所にある。{p().failed}
              </span>
            </Show>
            <Show when={p().leftover}>
              <span class="place-warn" role="alert">
                <Icon svg={alertTriangle} size={14} />
                手帳は移したが、前の場所 {p().leftover} に残ったファイルがある。中を確かめてから消す。
              </span>
            </Show>
          </>
        )}
      </Show>
      <form
        class="place-add"
        onSubmit={(e) => {
          e.preventDefault();
          if (path().trim()) void move(path().trim());
        }}
      >
        <div class="place-row">
          <input
            class="input mono"
            autocomplete="off"
            data-1p-ignore
            data-lpignore="true"
            data-bwignore
            aria-label="手帳の新しい場所"
            placeholder="/ か ~/ から書く"
            required
            value={path()}
            onInput={(e) => setPath(e.currentTarget.value)}
          />
          <button class="btn small primary" disabled={!!moving()}>
            移す
          </button>
        </div>
      </form>
    </section>
  );
}

export default function Secretary() {
  setView("設定 · 秘書");
  const persona = createAsync(() => personaQuery());
  return (
    <>
      <h1>秘書</h1>
      <p class="sub">
        あなたと話すエージェント。どのチャネルからの頼みにも応じ、手帳を読み書きし、外へ働きかけ、手代に仕事を委ねる。ここでは考えるのに使うモデル、呼び名と口調、書ける場所、手帳の場所を決める。
      </p>
      <div class="prefs">
        <RoleModel role={Role.SECRETARY} cap="上限は目安で、超えても会話は止めない。値段のあるモデルは円で、ないモデルは回数で数える。" />
        <Show when={persona()}>
          {(p) => (
            <>
              <NameSection name={p().name} />
              <ToneSection tone={p().tone} own={p().ownTone} />
            </>
          )}
        </Show>
        <WritableSection />
        <NotebookSection />
        <NotebookCheck />
      </div>
      <LogSheet title="秘書のログ">
        <SecretaryLog />
      </LogSheet>
    </>
  );
}
