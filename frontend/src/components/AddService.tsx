import { Collapsible } from "@ark-ui/solid/collapsible";
import { Dialog } from "@ark-ui/solid/dialog";
import { createAsync, useNavigate } from "@solidjs/router";
import { createMemo, createSignal, For, Match, Show, Switch } from "solid-js";
import Icon from "~/components/Icon";
import Modal from "~/components/Modal";
import Segs from "~/components/Segs";
import type { AccountField, ConnectResult } from "~/gen/famulen/v1/services_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { definitionsQuery, docIn, docUrl, iconNamed, knownQuery } from "~/lib/connections";
import * as icons from "~/lib/icons";
import { looksLikeTarget, type ServiceTarget, serviceTargets } from "~/lib/serviceTarget";

/** Something to choose in the dialog, from the known list or a definition. */
type Choice = {
  id: string;
  name: string;
  what: string;
  icon: string;
  definition: boolean;
  unavailable: string;
  fields: AccountField[];
  /** What its key is called, when it is added with one. */
  keyLabel: string;
  /** Where what it needs beforehand is written. */
  setup: string;
};

/** Keeps a password manager from taking a field for a login. */
const PLAIN = {
  autocomplete: "off",
  "data-1p-ignore": true,
  "data-lpignore": "true",
  "data-bwignore": true,
} as const;

/** The words people also look a service up by. */
const ALSO: Record<string, string> = {
  adroit: "アドロイト claude codex 実装",
  atlassian: "ジラ コンフルエンス",
  bluesky: "ブルースカイ bsky",
  clickup: "クリックアップ",
  discord: "ディスコード",
  docbase: "ドックベース",
  freee: "フリー 会計 人事 労務",
  github: "ギットハブ git",
  google: "グーグル gmail ジーメール calendar カレンダー",
  kibela: "キベラ",
  kingoftime: "キングオブタイム kot 勤怠",
  kintone: "キントーン cybozu サイボウズ",
  linear: "リニア",
  notion: "ノーション",
  slack: "スラック",
  slite: "スライト",
  todoist: "トゥドゥイスト todo",
};

const fold = (text: string) => text.normalize("NFKC").toLowerCase();

/** What a connection asks of the owner when it cannot be added yet. */
export function Unavailable(props: { text: string }) {
  return (
    <span class="sub">
      {props.text}
      <Show when={docIn(props.text)}>
        {(doc) => (
          <>
            {" "}
            <a href={docUrl(doc())} target="_blank" rel="noopener">
              {doc()}
            </a>
          </>
        )}
      </Show>
    </span>
  );
}

/** Adds a service: found by searching, or connected from what its instructions say. */
export default function AddService() {
  const navigate = useNavigate();
  const known = createAsync(() => knownQuery());
  const listed = createAsync(() => definitionsQuery());
  const [open, setOpen] = createSignal(false);
  const [search, setSearch] = createSignal("");
  const [chosen, setChosen] = createSignal<Choice>();
  const [custom, setCustom] = createSignal<string>();

  const choices = createMemo<Choice[]>(() => {
    const definitions = listed()?.definitions ?? [];
    const defined = new Set(definitions.map((d) => d.id));
    return [
      ...definitions.map((d) => ({
        id: d.id,
        name: d.name,
        what: d.about,
        icon: `brand-${d.id}`,
        definition: true,
        unavailable: d.appMissing,
        fields: d.fields,
        keyLabel: d.keyLabel,
        setup: d.setup,
      })),
      ...(known() ?? [])
        .filter((k) => !k.definition && !defined.has(k.id))
        .map((k) => ({
          id: k.id,
          name: k.name,
          what: k.what,
          icon: k.icon,
          definition: false,
          unavailable: k.available ? "" : k.unavailable,
          fields: k.fields,
          keyLabel: "",
          setup: "",
        })),
    ].sort((a, b) => a.name.localeCompare(b.name));
  });
  const found = () => {
    const terms = fold(search()).split(/\s+/).filter(Boolean);
    return choices().filter((c) => {
      const text = fold(`${c.id} ${c.name} ${c.what} ${ALSO[c.id] ?? ""}`);
      return terms.every((t) => text.includes(t));
    });
  };

  const added = (r: ConnectResult | undefined) => {
    if (!r) return;
    if (r.loginUrl) {
      location.href = r.loginUrl;
      return;
    }
    setOpen(false);
    if (r.connection) navigate(`/settings/services?id=${encodeURIComponent(r.connection.id)}`);
  };
  const connect = async (c: Choice, values: Record<string, string> = {}, apiKey = "") => {
    const env = Object.entries(values)
      .filter(([, v]) => v !== "")
      .map(([k, v]) => `${k}=${v}`)
      .join("\n");
    const consoleOrigin = location.origin;
    added(
      await act(
        () =>
          c.definition
            ? api.connections.addConnection({ target: c.id, how: "definition", env, apiKey, consoleOrigin })
            : api.connections.connectKnown({ service: c.id, env, consoleOrigin }),
        "つないだ",
      ),
    );
  };
  const choose = (c: Choice) => {
    if (c.unavailable || c.keyLabel || c.fields.length > 0) setChosen(c);
    else void connect(c);
  };
  const back = () => {
    setChosen(undefined);
    setCustom(undefined);
  };

  return (
    <Modal
      open={open()}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) {
          setSearch("");
          back();
        }
      }}
      title="サービスを足す"
      size="wide"
      trigger={
        <Dialog.Trigger class="btn primary">
          <Icon svg={icons.plus} size={14} />
          サービスを足す
        </Dialog.Trigger>
      }
    >
      <Switch>
        <Match when={chosen()} keyed>
          {(c) => <Fill choice={c} onBack={back} onAdd={(values, key) => connect(c, values, key)} />}
        </Match>
        <Match when={custom() !== undefined}>
          <Custom text={custom() ?? ""} onBack={back} onAdded={added} />
        </Match>
        <Match when={true}>
          <div class="service-add">
            <label class="service-search">
              <Icon svg={icons.search} size={16} />
              <input
                {...PLAIN}
                class="input"
                type="search"
                aria-label="サービスを探す"
                placeholder="サービスの名前か、URL・コマンド"
                autofocus
                value={search()}
                onInput={(e) => setSearch(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key !== "Enter" || e.isComposing) return;
                  e.preventDefault();
                  const [only, ...others] = found();
                  if (only && others.length === 0) choose(only);
                  else if (!only && serviceTargets(search()).length > 0) setCustom(search());
                }}
              />
            </label>
            <div class="service-choices" role="list" aria-label="足せるサービス">
              <For each={found()}>
                {(c) => (
                  <div role="listitem">
                    <button type="button" class="service-choice" onClick={() => choose(c)}>
                      <span class="index-icon">
                        <Icon svg={iconNamed(c.icon)} size={20} />
                      </span>
                      <span class="grow">
                        <b>{c.name}</b>
                        <Show when={c.what}>
                          <span class="sub">{c.what}</span>
                        </Show>
                      </span>
                      <Show when={c.unavailable}>
                        <span class="tag">要アプリ登録</span>
                      </Show>
                    </button>
                  </div>
                )}
              </For>
            </div>
            <Show when={found().length === 0}>
              <p class="sub">一覧に「{search()}」はない。</p>
            </Show>
            <For each={listed()?.broken ?? []}>{(b) => <p class="sub warn">読めなかった定義：{b}</p>}</For>
            <button
              type="button"
              class="service-choice"
              onClick={() => setCustom(found().length === 0 || looksLikeTarget(search()) ? search() : "")}
            >
              <span class="index-icon">
                <Icon svg={icons.plugConnected} size={20} />
              </span>
              <span class="grow">
                <b>一覧にないサービスをつなぐ</b>
                <span class="sub">そのサービスの説明にある URL・コマンド・設定を貼るだけ</span>
              </span>
              <Icon svg={icons.chevronRight} size={14} />
            </button>
          </div>
        </Match>
      </Switch>
    </Modal>
  );
}

function Back(props: { onBack: () => void }) {
  return (
    <button type="button" class="btn" onClick={props.onBack}>
      <Icon svg={icons.chevronLeft} size={14} />
      一覧へ
    </button>
  );
}

/** The values a definition asks for, or why it cannot be added yet. */
function Fill(props: { choice: Choice; onBack: () => void; onAdd: (values: Record<string, string>, key: string) => Promise<void> }) {
  const [values, setValues] = createSignal<Record<string, string>>({});
  const [key, setKey] = createSignal("");
  const c = props.choice;
  return (
    <form
      class="dialog-form"
      onSubmit={(e) => {
        e.preventDefault();
        void props.onAdd(values(), key());
      }}
    >
      <div class="row">
        <span class="index-icon">
          <Icon svg={iconNamed(c.icon)} size={20} />
        </span>
        <div class="grow">
          <b>{c.name}</b>
          <p class="sub">{c.what}</p>
        </div>
      </div>
      <Show
        when={!c.unavailable}
        fallback={
          <p>
            <Unavailable text={c.unavailable} />
          </p>
        }
      >
        <For each={c.fields}>
          {(f) => (
            <label class="dialog-field">
              <span>
                {f.label}
                {f.optional ? "（なくてもよい）" : ""}
              </span>
              <input
                {...PLAIN}
                class="input"
                type={f.secret ? "password" : "text"}
                required={!f.optional}
                value={values()[f.name] ?? ""}
                onInput={(e) => {
                  const v = e.currentTarget.value;
                  setValues((all) => ({ ...all, [f.name]: v }));
                }}
              />
            </label>
          )}
        </For>
        <Show when={c.keyLabel}>
          <label class="dialog-field">
            <span>{c.keyLabel}</span>
            <input
              {...PLAIN}
              class="input"
              type="password"
              required
              value={key()}
              onInput={(e) => setKey(e.currentTarget.value)}
            />
          </label>
        </Show>
        <Show when={c.setup}>
          {(place) => (
            <a class="sub" href={/^https?:/.test(place()) ? place() : docUrl(place())} target="_blank" rel="noopener">
              つなぐ前に要るもの
            </a>
          )}
        </Show>
      </Show>
      <div class="dialog-actions">
        <Back onBack={props.onBack} />
        <Show when={!c.unavailable}>
          <button class="btn primary">足す</button>
        </Show>
      </div>
    </form>
  );
}

const WAY: Record<ServiceTarget["how"], { label: string; note: string }> = {
  remote: { label: "リモートの MCP サーバー", note: "ログインの要るサービスなら、このあとログインの画面に進む。" },
  api: { label: "API", note: "秘書が HTTP で直接呼ぶ。" },
  local: {
    label: "このマシンで動かす MCP サーバー",
    note: "このコマンドを、あなたの権限でこのマシンで動かす。出どころの確かなものだけを足す。",
  },
};

/** A service connected from what its instructions say to paste. */
function Custom(props: { text: string; onBack: () => void; onAdded: (r: ConnectResult | undefined) => void }) {
  const [text, setText] = createSignal(props.text);
  const [at, setAt] = createSignal(0);
  const targets = createMemo(() => serviceTargets(text()));
  const current = createMemo(() => targets()[at()] ?? targets()[0]);
  return (
    <div class="dialog-form">
      <label class="dialog-field">
        <span>URL・コマンド・設定を貼る</span>
        <textarea
          {...PLAIN}
          class="input mono service-paste"
          autofocus
          spellcheck={false}
          placeholder={"https://mcp.example.com/mcp\nnpx -y some-mcp-server\n{ \"mcpServers\": { … } }"}
          value={text()}
          onInput={(e) => {
            setText(e.currentTarget.value);
            setAt(0);
          }}
        />
        <span class="sub">
          サービスの説明の「MCP」の節に、ほかの AI アプリ（Claude など）に入れるものとして書かれている。そのまま貼ると、つなぎ方を読み取る。
        </span>
      </label>
      <Show when={targets().length > 1}>
        <div class="dialog-field">
          <span>どれをつなぐか</span>
          <Segs
            label="どれをつなぐか"
            items={targets().map((t, i) => [String(i), t.name || t.target])}
            value={String(at())}
            onChoose={(v) => setAt(Number(v))}
          />
        </div>
      </Show>
      <Show
        when={current()}
        keyed
        fallback={
          <>
            <Show when={text().trim()}>
              <p class="sub warn">つなぎ方を読み取れない。URL かコマンドだけを貼ってみる。</p>
            </Show>
            <div class="dialog-actions">
              <Back onBack={props.onBack} />
              <button type="button" class="btn primary" disabled>
                つなぐ
              </button>
            </div>
          </>
        }
      >
        {(t) => <Details read={t} onBack={props.onBack} onAdded={props.onAdded} />}
      </Show>
    </div>
  );
}

function Details(props: { read: ServiceTarget; onBack: () => void; onAdded: (r: ConnectResult | undefined) => void }) {
  const t = props.read;
  const [how, setHow] = createSignal(t.how);
  const [name, setName] = createSignal(t.name);
  const [openapi, setOpenapi] = createSignal(t.openapi);
  const [key, setKey] = createSignal(t.key);
  const [header, setHeader] = createSignal(t.header);
  const [env, setEnv] = createSignal(t.env);
  const [busy, setBusy] = createSignal(false);
  const setVariable = (i: number, part: 0 | 1, value: string) =>
    setEnv((all) => all.map((v, n) => (n === i ? (part === 0 ? [value, v[1]] : [v[0], value]) : v)));
  const add = async (e: SubmitEvent) => {
    e.preventDefault();
    setBusy(true);
    const r = await act(
      () =>
        api.connections.addConnection({
          target: t.target,
          how: how() === "api" ? "api" : "",
          openapi: how() === "api" ? openapi() : "",
          name: name(),
          apiKey: how() === "local" ? "" : key(),
          header: header(),
          env: env()
            .filter(([k]) => k.trim())
            .map(([k, v]) => `${k.trim()}=${v}`)
            .join("\n"),
          consoleOrigin: location.origin,
        }),
      "つないだ",
    );
    setBusy(false);
    props.onAdded(r);
  };
  return (
    <form class="dialog-form" onSubmit={add}>
      <div class="service-read">
        <div class="row">
          <Icon svg={how() === "local" ? icons.terminal : icons.world} size={18} />
          <b class="grow">{WAY[how()].label}</b>
          <Show when={t.how !== "local"}>
            <Segs
              label="つなぎ方"
              items={[
                ["remote", "MCP"],
                ["api", "API"],
              ]}
              value={how()}
              onChoose={(v) => setHow(v === "api" ? "api" : "remote")}
            />
          </Show>
        </div>
        <span class="mono sub service-target">{t.target}</span>
        <span class="sub">{WAY[how()].note}</span>
      </div>
      <label class="dialog-field">
        <span>名前（なくてもよい）</span>
        <input
          {...PLAIN}
          class="input"
          placeholder="貼ったものから付ける"
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
      </label>
      <Show when={how() === "local"}>
        <div class="dialog-field">
          <span>環境変数</span>
          <span class="sub">鍵などを渡すなら入れる。値は金庫に入る。</span>
          <For each={env().map((_, i) => i)}>
            {(i) => (
              <div class="row service-env">
                <input
                  {...PLAIN}
                  class="input mono"
                  aria-label={`環境変数 ${i + 1} の名前`}
                  placeholder="API_KEY"
                  value={env()[i]?.[0] ?? ""}
                  onInput={(e) => setVariable(i, 0, e.currentTarget.value)}
                />
                <input
                  {...PLAIN}
                  class="input grow"
                  type="password"
                  aria-label={env()[i]?.[0] ? `${env()[i]?.[0]} の値` : `環境変数 ${i + 1} の値`}
                  value={env()[i]?.[1] ?? ""}
                  onInput={(e) => setVariable(i, 1, e.currentTarget.value)}
                />
              </div>
            )}
          </For>
          <button type="button" class="btn small service-more" onClick={() => setEnv((all) => [...all, ["", ""]])}>
            <Icon svg={icons.plus} size={14} />
            環境変数を足す
          </button>
        </div>
      </Show>
      <Show when={how() !== "local"}>
        <Collapsible.Root defaultOpen={how() === "api" || t.key !== "" || t.header !== ""}>
          <Collapsible.Trigger class="vault-auto-trigger">
            <Icon svg={icons.chevronRight} size={14} />
            鍵でつなぐ
          </Collapsible.Trigger>
          <Collapsible.Content class="vault-auto-content">
            <div class="dialog-form">
              <label class="dialog-field">
                <span>鍵（API キー・トークン）</span>
                <input
                  {...PLAIN}
                  class="input"
                  type="password"
                  value={key()}
                  onInput={(e) => setKey(e.currentTarget.value)}
                />
                <span class="sub">サービスの画面で発行したもの。金庫に入る。ログインでつなぐなら要らない。</span>
              </label>
              <label class="dialog-field">
                <span>鍵を入れるヘッダー</span>
                <input
                  {...PLAIN}
                  class="input mono"
                  placeholder="Authorization"
                  value={header()}
                  onInput={(e) => setHeader(e.currentTarget.value)}
                />
                <span class="sub">サービスの説明に X-Api-Key などと書かれているときだけ変える。</span>
              </label>
              <Show when={how() === "api"}>
                <label class="dialog-field">
                  <span>OpenAPI の URL（なくてもよい）</span>
                  <input
                    {...PLAIN}
                    class="input mono"
                    value={openapi()}
                    onInput={(e) => setOpenapi(e.currentTarget.value)}
                  />
                </label>
              </Show>
            </div>
          </Collapsible.Content>
        </Collapsible.Root>
      </Show>
      <div class="dialog-actions">
        <Back onBack={props.onBack} />
        <button class="btn primary" disabled={busy()}>
          つなぐ
        </button>
      </div>
    </form>
  );
}
