import { Collapsible } from "@ark-ui/solid/collapsible";
import { A, createAsync, useNavigate, useSearchParams } from "@solidjs/router";
import { createSignal, For, type JSX, Show } from "solid-js";
import AddService from "~/components/AddService";
import DefinitionDiff from "~/components/DefinitionDiff";
import Icon from "~/components/Icon";
import PermissionSeg, { PERMISSIONS } from "~/components/PermissionSeg";
import { CalendarsSection, ChannelsSection, LookoutSection, Pref } from "~/components/ServiceSections";
import {
  type Connection,
  type ConnectionDetail,
  ConnectionState,
  type DefinitionChange,
  Permission,
  type ToolSetting,
  TransportKind,
} from "~/gen/famulen/v1/services_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import {
  builtinToolsQuery,
  channelsQuery,
  connectionQuery,
  connectionsQuery,
  DESTINATION_LABEL,
  definitionChangesQuery,
  iconNamed,
  kindQuery,
  knownQuery,
} from "~/lib/connections";
import { personaQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { shortTime } from "~/lib/time";
import { setSelection, setView } from "~/lib/view";

const OWN = "famulen";
const GOOGLE = "google";
const SLACK = "slack";

const col = (gap: string): JSX.CSSProperties => ({ display: "flex", "flex-direction": "column", gap });
const plain: JSX.CSSProperties = { color: "inherit", "text-decoration": "none" };

/**
 * The owner's accounts on outside services, one of them with its tools, or
 * the tool defaults a kind of service gives all its accounts.
 */
export default function Services() {
  const [params] = useSearchParams();
  const id = () => (typeof params.id === "string" ? params.id : "");
  const kind = () => (typeof params.kind === "string" ? params.kind : "");
  const builtin = () => params.builtin === "1";
  return (
    <Show
      when={id()}
      fallback={
        <Show when={!builtin()} fallback={<BuiltinTools />}>
          <Show when={kind()} fallback={<ServiceList />} keyed>
            {(k) => <KindDetail kind={k} />}
          </Show>
        </Show>
      }
      keyed
    >
      {(i) => <ServiceDetail id={i} />}
    </Show>
  );
}

const kindHref = (kind: string) => `/settings/services?kind=${encodeURIComponent(kind)}`;
const KIND_SOURCE = "種類の既定";

function ConnectionRow(props: { c: Connection; icon: string }) {
  const sub = () => [props.c.account && props.c.account !== props.c.name ? props.c.account : "", props.c.watching].filter(Boolean).join(" · ");
  return (
    <A class="item" href={`/settings/services?id=${encodeURIComponent(props.c.id)}`} style={plain}>
      <Icon svg={props.icon} size={18} />
      <div class="grow">
        <b>{props.c.name}</b>
        <Show when={sub()}>
          <br />
          <span class="sub">{sub()}</span>
        </Show>
      </div>
      <span class={`tag${props.c.state !== ConnectionState.CONNECTED ? " warn" : ""}`}>{props.c.status}</span>
      <Icon svg={icons.chevronRight} size={14} />
    </A>
  );
}

async function goLogin(run: () => Promise<{ loginUrl: string } | undefined>) {
  const r = await run();
  if (r?.loginUrl) location.href = r.loginUrl;
}

function ServiceList() {
  setView("設定 · サービス");
  const list = createAsync(() => connectionsQuery());
  const known = createAsync(() => knownQuery());
  const persona = createAsync(() => personaQuery());
  const [search, setSearch] = createSignal("");
  const connections = () => list()?.connections ?? [];
  const found = () => {
    const terms = search().normalize("NFKC").toLowerCase().split(/\s+/).filter(Boolean);
    return connections()
      .filter((c) => terms.every((t) => `${c.name} ${c.kind} ${c.account}`.normalize("NFKC").toLowerCase().includes(t)))
      .sort((a, b) => a.name.localeCompare(b.name) || a.account.localeCompare(b.account));
  };
  const iconOf = (kind: string) => iconNamed(known()?.find((k) => k.id === kind)?.icon ?? `brand-${kind}`);
  const nameOf = (kind: string) => known()?.find((k) => k.id === kind)?.name ?? kind;
  const kinds = () => [...new Set(connections().map((c) => c.kind).filter((k) => k))].sort();

  return (
    <>
      <div class="row">
        <h1 class="grow">サービス</h1>
        <AddService />
      </div>
      <p class="sub">
        あなたのアカウントや端末をつなぐ。つないだものは秘書が使い、カレンダー・見張り・チャネルにも入る。同じサービスの別のアカウントも、いくつでも足せる。
      </p>
      <section style={col("8px")}>
        <h2>つないだサービス</h2>
        <Show when={connections().length > 0} fallback={<p class="sub">まだ何もつないでいない。</p>}>
          <label class="service-search">
            <Icon svg={icons.search} size={16} />
            <input
              class="input"
              type="search"
              autocomplete="off"
              aria-label="つないだサービスを探す"
              placeholder="名前・アカウントで探す"
              value={search()}
              onInput={(e) => setSearch(e.currentTarget.value)}
            />
          </label>
          <div class="list">
            <For each={found()} fallback={<p class="sub">「{search()}」に当たるものはない。</p>}>
              {(c) => <ConnectionRow c={c} icon={iconOf(c.kind)} />}
            </For>
          </div>
        </Show>
      </section>
      <Show when={kinds().length > 0}>
        <section style={col("8px")}>
          <h2>道具の権限の既定</h2>
          <p class="sub">
            道具を秘書にどこまで使わせるかを、サービスごとにまとめて決める。同じサービスのどのアカウントにも効き、アカウントごとに上書きもできる。
          </p>
          <div class="list">
            <For each={kinds()}>
              {(k) => (
                <A class="item" href={kindHref(k)} style={plain}>
                  <Icon svg={iconOf(k)} size={18} />
                  <div class="grow">
                    <b>{nameOf(k)}</b>
                    <br />
                    <span class="sub">
                      {connections()
                        .filter((c) => c.kind === k)
                        .map((c) => c.account || c.name)
                        .join("、")}{" "}
                      に効く
                    </span>
                  </div>
                  <Icon svg={icons.chevronRight} size={14} />
                </A>
              )}
            </For>
          </div>
        </section>
      </Show>
      <section style={col("8px")}>
        <h2>そのほか</h2>
        <div class="list">
          <A class="item" href={`/settings/services?id=${OWN}`} style={plain}>
            <Icon svg={icons.shieldCheck} size={18} />
            <div class="grow">
              <b>{persona()?.name ?? "Famulen"} 自身の道具</b>
              <br />
              <span class="sub">会話・記録・記憶・予定・Slack</span>
            </div>
            <Icon svg={icons.chevronRight} size={14} />
          </A>
          <A class="item" href="/settings/services?builtin=1" style={plain}>
            <Icon svg={icons.terminal} size={18} />
            <div class="grow">
              <b>秘書の組み込みの道具</b>
              <br />
              <span class="sub">ファイルを読む・書く・探す、シェル</span>
            </div>
            <Icon svg={icons.chevronRight} size={14} />
          </A>
        </div>
      </section>
      <DefinitionChanges />
    </>
  );
}

const CHANGE_KIND: Record<string, string> = { change: "変えた", revert: "戻した" };

/**
 * The changes that went into the service definitions, by definition, each
 * opening to its diff. The newest change of a definition can be taken back.
 */
function DefinitionChanges() {
  const changes = createAsync(() => definitionChangesQuery());
  const persona = createAsync(() => personaQuery());
  const tz = () => persona()?.timezone ?? "UTC";
  const byDefinition = () => {
    const groups = new Map<string, DefinitionChange[]>();
    for (const c of changes() ?? []) groups.set(c.definition, [...(groups.get(c.definition) ?? []), c]);
    return [...groups.entries()];
  };
  const revert = (id: string) =>
    void act(() => api.connections.revertDefinition({ id }), "一つ前に戻した");
  return (
    <Show when={byDefinition().length > 0}>
      <section style={col("8px")}>
        <h2>定義の変更</h2>
        <p class="sub">許して入ったサービス定義の変更。一番新しい変更は、一つ前に戻せる。戻したことも変更として残る。</p>
        <div class="prefs">
          <For each={byDefinition()}>
            {([definition, list]) => (
              <div class="pref" data-testid="definition-changes">
                <div class="row">
                  <b class="grow mono">{definition}</b>
                  <button
                    type="button"
                    class="btn small"
                    aria-label={`${definition}を一つ前に戻す`}
                    onClick={() => revert(definition)}
                  >
                    <Icon svg={icons.arrowBackUp} size={14} />
                    一つ前に戻す
                  </button>
                </div>
                <For each={list}>
                  {(c) => (
                    <Collapsible.Root class="pref-sub">
                      <Collapsible.Trigger class="change-trigger">
                        <span class="mono sub">{shortTime(c.at, tz())}</span>
                        <span class="tag">{CHANGE_KIND[c.kind] ?? c.kind}</span>
                        <span class="sub">{c.actor}</span>
                        <span class="grow">{c.why}</span>
                        <Icon svg={icons.chevronDown} size={14} />
                      </Collapsible.Trigger>
                      <Collapsible.Content>
                        <DefinitionDiff before={c.before} after={c.after} />
                      </Collapsible.Content>
                    </Collapsible.Root>
                  )}
                </For>
              </div>
            )}
          </For>
        </div>
      </section>
    </Show>
  );
}

/**
 * Each receiver of a defined connection other than its channels, with where
 * what arrives goes. A channel is turned on and off in its own section.
 */
function Receivers(props: { detail: ConnectionDetail; id: string }) {
  const label = (name: string) => DESTINATION_LABEL[name] ?? name;
  const channels = createAsync(() => channelsQuery());
  const shown = () =>
    props.detail.receivers.filter(
      (r) => !(channels() ?? []).some((c) => c.connection === props.id && c.receiver === r.name),
    );
  return (
    <Show when={shown().length > 0}>
      <div class="pref-sub">
        <h3>受け口</h3>
        <p class="sub">外から届いたものを、手を加えずにどこへ渡すか。</p>
        <div class="list">
          <For each={shown()}>
            {(r) => (
              <div class="item">
                <div class="grow">
                  <span class="mono">{r.name}</span> <span class="sub">{r.how}</span>
                </div>
                <label class="sub" for={`rx-${r.name}`}>
                  渡す先
                </label>
                <select
                  id={`rx-${r.name}`}
                  class="input"
                  onChange={(e) => {
                    const destination = e.currentTarget.value;
                    void act(
                      () => api.connections.setDestination({ connection: props.id, receiver: r.name, destination }),
                      destination ? `${label(destination)}に渡す` : "受け口を閉じた",
                    );
                  }}
                >
                  <option value="" selected={!r.destination}>
                    どこにも渡さない（閉じる）
                  </option>
                  <For each={props.detail.destinations}>
                    {(d) => (
                      <option value={d} selected={d === r.destination}>
                        {label(d)}
                      </option>
                    )}
                  </For>
                </select>
              </div>
            )}
          </For>
        </div>
      </div>
    </Show>
  );
}

/** A defined connection's account values; a secret one is only replaced. */
function AccountValues(props: { detail: ConnectionDetail; id: string }) {
  const fields = () => props.detail.values.filter((v) => v.label);
  const found = () => props.detail.values.filter((v) => !v.label);
  return (
    <Show when={props.detail.values.length > 0}>
      <div class="pref-sub">
        <h3>アカウントの値</h3>
        <div class="list">
          <For each={fields()}>
            {(v) => {
              const [text, setText] = createSignal(v.secret ? "" : v.value);
              const save = async (e: SubmitEvent) => {
                e.preventDefault();
                const done = await act(
                  () => api.connections.setAccountValue({ connection: props.id, name: v.name, value: text() }),
                  v.secret ? "金庫に入れた" : "保存した",
                );
                if (done && v.secret) setText("");
              };
              return (
                <form class="item" onSubmit={save}>
                  <label class="grow" for={`av-${v.name}`}>
                    {v.label}
                    <Show when={v.secret}>
                      {" "}
                      <span class="tag">金庫</span>
                    </Show>
                  </label>
                  <input
                    id={`av-${v.name}`}
                    class="input"
                    type={v.secret ? "password" : "text"}
                    autocomplete="off"
                    placeholder={v.secret && v.set ? "入っている" : ""}
                    value={text()}
                    onInput={(e) => setText(e.currentTarget.value)}
                  />
                  <button class="btn small">{v.secret && v.set ? "入れ直す" : "保存"}</button>
                </form>
              );
            }}
          </For>
          <For each={found()}>
            {(v) => (
              <div class="item">
                <span class="grow mono">{v.name}</span>
                <span class="sub mono">{v.value}</span>
              </div>
            )}
          </For>
        </div>
      </div>
    </Show>
  );
}

const BULK: [string, string][] = [
  ["read", "読むだけの道具"],
  ["write", "書く・送る道具"],
];
const BULK_RULES: [Permission, string][] = [
  ...PERMISSIONS,
  [Permission.UNSPECIFIED, "既定に戻す"],
];

function BulkPermissions(props: { scope: string }) {
  return (
    <div class="card" style={{ ...col("6px"), padding: "10px 12px" }}>
      <span class="sub">まとめて選ぶ（押すとすぐ変わる）</span>
      <span class="sub">
        「まかせる」は、読むだけの道具はそのまま使い、消す・外の人へ送るなど取り返しのつかないものや人を巻き込むものは必ずあなたに確認し、それ以外は
        <A href="/settings/roles/assess">見極め</A>
        が頼まれたとおりかを判断する。「許可する」は尋ねずに使い、「確認する」は毎回あなたに尋ね、「拒否する」は使わせない。
      </span>
      <For each={BULK}>
        {([which, label]) => (
          <div class="row" style={{ gap: "6px" }}>
            <span style={{ width: "9em" }}>{label}を</span>
            <For each={BULK_RULES}>
              {([permission, l]) => (
                <button
                  type="button"
                  class="btn small"
                  onClick={() => act(() => api.connections.setPermissions({ scope: props.scope, which, permission }))}
                >
                  {l}
                </button>
              )}
            </For>
          </div>
        )}
      </For>
    </div>
  );
}

function ToolRow(props: { t: ToolSetting; scope: string; owner: string; readOnlyTag: boolean; source?: JSX.Element }) {
  return (
    <div class="item" onClick={() => setSelection(`${props.owner} の道具 ${props.t.name}`)}>
      <div class="grow">
        <span class="mono" style={{ "font-size": "13px" }}>
          {props.t.name}
        </span>{" "}
        {props.source}
        <Show when={props.t.readOnly && props.readOnlyTag}>
          {" "}
          <span class="tag confirmed">読むだけ</span>
        </Show>
        <Show when={props.t.alwaysAsk && props.t.effective === Permission.AUTO}>
          {" "}
          <span class="tag warn" title="取り返しがつかないか、人を巻き込むので、まかせても必ず確認する">
            必ず確認する
          </span>
        </Show>
        <Show when={props.t.description}>
          <br />
          <span class="sub">{[...props.t.description].slice(0, 120).join("")}</span>
        </Show>
      </div>
      <PermissionSeg
        tool={props.t.name}
        permission={props.t.permission}
        inherited={props.t.inherited}
        onChoose={(permission, done) =>
          void act(() => api.connections.setPermission({ scope: props.scope, tool: props.t.name, permission }), done)
        }
      />
    </div>
  );
}

/** The secretary's model CLI's built-in tools and the owner's permission for each. */
function BuiltinTools() {
  const tools = createAsync(() => builtinToolsQuery());
  setView("設定 · サービス · 組み込みの道具");
  return (
    <>
      <div class="row">
        <A class="btn small" href="/settings/services" aria-label="一覧へ">
          <Icon svg={icons.chevronLeft} size={14} />
        </A>
        <h1>秘書の組み込みの道具</h1>
      </div>
      <p class="sub">秘書のモデル CLI がはじめから持つ道具。どれを選んでも、ゲートウェイの置き場所と .env には届かない。</p>
      <BulkPermissions scope="" />
      <div class="list">
        <For each={tools() ?? []}>
          {(t) => <ToolRow t={t} scope="" owner="秘書の組み込み" readOnlyTag />}
        </For>
        <Show when={tools() && (tools() ?? []).length === 0}>
          <p class="empty">秘書がまだ動いていないので、道具の一覧がない</p>
        </Show>
      </div>
    </>
  );
}

function KindDetail(props: { kind: string }) {
  const detail = createAsync(() => kindQuery(props.kind));
  const scope = () => `kind:${props.kind}`;
  setView(`設定 · サービス種 · ${props.kind}`);
  return (
    <>
      <div class="row">
        <A class="btn small" href="/settings/services" aria-label="一覧へ">
          <Icon svg={icons.chevronLeft} size={14} />
        </A>
        <h1>サービス種「{props.kind}」の既定</h1>
      </div>
      <p class="sub">同じ種類のつなぎ先すべてに効く。つなぎ先ごとの決まりがあれば、そちらが勝つ。</p>
      <BulkPermissions scope={scope()} />
      <div class="row">
        <span class="sub">この既定を使っているつなぎ先：</span>
        <For each={detail()?.connections ?? []}>
          {(c) => (
            <A class="tag" href={`/settings/services?id=${encodeURIComponent(c.id)}`}>
              {c.account ? `${c.name} · ${c.account}` : c.name}
            </A>
          )}
        </For>
      </div>
      <div class="list">
        <For each={detail()?.tools ?? []}>
          {(t) => (
            <ToolRow
              t={t}
              scope={scope()}
              owner={`サービス種「${props.kind}」`}
              readOnlyTag
            />
          )}
        </For>
        <Show when={detail() && (detail()?.tools ?? []).length === 0}>
          <p class="empty">この種類のつなぎ先は、まだ道具を読んでいない</p>
        </Show>
      </div>
    </>
  );
}

function ServiceDetail(props: { id: string }) {
  const detail = createAsync(() => connectionQuery(props.id));
  const c = () => detail()?.connection;
  const scope = () => `conn:${props.id}`;
  const own = () => props.id === OWN;
  const ours = () => !own();
  // Google and Slack accounts come from Famulen's own apps: their name, kind,
  // address and keys are set by the login, so there is nothing to type.
  const managed = () => c()?.kind === GOOGLE || c()?.kind === SLACK;
  const [key, setKey] = createSignal("");
  const [header, setHeader] = createSignal("");
  /** The definition says what the account is, so it is not the owner's to write. */
  const defined = () => c()?.transportKind === TransportKind.DEFINITION && Boolean(c()?.account);
  const navigate = useNavigate();
  const persona = createAsync(() => personaQuery());

  setView(`設定 · サービス · ${props.id}`);

  const edit = (field: "name" | "kind" | "account", value: string) =>
    act(() =>
      api.connections.editConnection(
        field === "name" ? { id: props.id, name: value } : field === "kind" ? { id: props.id, kind: value } : { id: props.id, account: value },
      ),
    );

  const setSecret = async (e: SubmitEvent) => {
    e.preventDefault();
    const done = await act(
      () => api.connections.setSecret({ id: props.id, apiKey: key(), header: header() }),
      "鍵を入れた",
    );
    if (done) setKey("");
  };

  const login = () =>
    goLogin(async () => {
      const go = await act(() => api.connections.startLogin({ id: props.id, consoleOrigin: location.origin }), "");
      return go && { loginUrl: go.url };
    });

  return (
    <Show when={c()} fallback={<p class="sub">読んでいる</p>}>
      {(conn) => (
        <>
          <div class="row">
            <A class="btn small" href="/settings/services" aria-label="一覧へ">
              <Icon svg={icons.chevronLeft} size={14} />
            </A>
            <h1>{conn().name}</h1>
            <span class="tag">{conn().transport}</span>
            <Show when={ours()}>
              <span class="sub">{conn().account}</span>
              <span class="tag" style={{ "margin-left": "auto" }}>
                {conn().status}
              </span>
            </Show>
          </div>
          <Show when={own()}>
            <p class="sub">
              会話・記録・記憶・予定・Slack への投稿など、Famulen が自分で持つ道具。ほかのつなぎ先と同じように任せ方を決められる。
            </p>
          </Show>
          <div class="prefs">
            <Show when={ours()}>
              <Pref
                title="つながり"
                note={
                  managed()
                    ? conn().kind === GOOGLE
                      ? "Famulen の Google アプリでログインしたアカウント。名前や鍵を入れる必要はない。"
                      : "サービスで足したワークスペース。bot と、あなたのアカウントで読む分が一緒につながっている。"
                    : "どこへ、どのアカウントでつなぐか。外すと、このつなぎ先の道具・チャネル・見張りがすべてなくなる。"
                }
              >
                <Show when={!managed()}>
                  <div class="connection-fields">
                    <label class="dialog-field">
                      <span>名前</span>
                      <input class="input" value={conn().name} required onChange={(e) => edit("name", e.currentTarget.value)} />
                      <span class="sub">一覧や見張りでの呼び名。好きに変えてよく、つながりは変わらない。</span>
                    </label>
                    <Show
                      when={defined()}
                      fallback={
                        <label class="dialog-field">
                          <span>アカウントの目印</span>
                          <input
                            class="input"
                            value={conn().account}
                            onChange={(e) => edit("account", e.currentTarget.value)}
                          />
                          <span class="sub">
                            誰のアカウントかを見分けるための書き置き。サービスから分かれば初めから入る。書き換えても、つながるアカウントは変わらない。
                          </span>
                        </label>
                      }
                    >
                      <div class="dialog-field">
                        <span>アカウント</span>
                        <div>{conn().account}</div>
                        <span class="sub">つないだアカウント。ここでは変えられない。別のアカウントを使うなら、サービスを足す。</span>
                      </div>
                    </Show>
                  </div>
                </Show>
                <Show when={detail()?.login && conn().state === ConnectionState.CONNECTED}>
                  <p class="sub">
                    つながっているので、ログインは要らない。切れたときや、許す範囲を変えるときにだけログインし直す。
                  </p>
                </Show>
                <div class="row">
                  <Show when={detail()?.login}>
                    <button type="button" class="btn" classList={{ primary: conn().state !== ConnectionState.CONNECTED }} onClick={login}>
                      <Icon svg={icons.login2} size={14} />
                      {conn().state === ConnectionState.LOGIN_PENDING ? "ログインを続ける" : "ログインし直す"}
                    </button>
                  </Show>
                  <button
                    type="button"
                    class="btn danger"
                    onClick={() => {
                      // Leave first: reloading the page of a removed connection would fail.
                      navigate("/settings/services");
                      void act(() => api.connections.deleteConnection({ id: props.id }), "外した");
                    }}
                  >
                    <Icon svg={icons.trash} size={14} />
                    外す
                  </button>
                </div>
                <Show when={!managed() && conn().transportKind === TransportKind.API}>
                  <form class="row" onSubmit={setSecret}>
                    <input
                      class="input"
                      placeholder="Authorization"
                      aria-label="鍵を入れるヘッダー"
                      value={header()}
                      onInput={(e) => setHeader(e.currentTarget.value)}
                    />
                    <input
                      class="input grow"
                      type="password"
                      autocomplete="off"
                      placeholder={detail()?.hasSecret ? "金庫の API キーを置き換える" : "API キー"}
                      aria-label="API キー"
                      value={key()}
                      onInput={(e) => setKey(e.currentTarget.value)}
                    />
                    <button class="btn small">鍵を入れる</button>
                  </form>
                </Show>
                <Show when={detail()}>
                  {(d) => (
                    <>
                      <Receivers detail={d()} id={props.id} />
                      <AccountValues detail={d()} id={props.id} />
                    </>
                  )}
                </Show>
                <Show when={!managed()}>
                  <Collapsible.Root>
                    <Collapsible.Trigger class="vault-auto-trigger">
                      <Icon svg={icons.chevronRight} size={14} />
                      詳しい設定
                    </Collapsible.Trigger>
                    <Collapsible.Content class="vault-auto-content">
                      <div class="connection-fields">
                        <Show when={detail()?.target}>
                          <div class="dialog-field">
                            <span>{conn().transport}</span>
                            <div class="mono service-target">{detail()?.target}</div>
                          </div>
                        </Show>
                        <label class="dialog-field">
                          <span>サービス種</span>
                          <input
                            class="input mono"
                            value={conn().kind}
                            required
                            onChange={(e) => edit("kind", e.currentTarget.value)}
                          />
                          <span class="sub">
                            同じサービス種のつなぎ先は、道具の権限の既定を分け合う。変えると、新しい種類の既定に従う。ふつうは変えない。
                          </span>
                        </label>
                      </div>
                    </Collapsible.Content>
                  </Collapsible.Root>
                </Show>
              </Pref>
              <ChannelsSection connection={props.id} />
              <Show when={conn().kind === GOOGLE}>
                <CalendarsSection
                  connection={props.id}
                  account={conn().account || conn().name}
                  timezone={persona()?.timezone ?? "UTC"}
                />
              </Show>
              <LookoutSection connection={props.id} />
            </Show>
            <Pref
              title="道具"
              note={
                <Show when={ours()} fallback="このつなぎ先の道具を、秘書にどこまで使わせるか。">
                  いまある道具と、どこまで使わせるか。道具ごとに、サービス種「{conn().kind}」の既定を使うか上書きするかを選ぶ。
                  <Show when={conn().kind}>
                    {" "}
                    <A href={kindHref(conn().kind)}>サービス種「{conn().kind}」の既定を見る</A>
                  </Show>
                </Show>
              }
              action={
                <Show when={ours() && !managed()}>
                  <button
                    type="button"
                    class="btn small"
                    onClick={() => act(() => api.connections.refreshTools({ id: props.id }), "道具を読み直した")}
                  >
                    <Icon svg={icons.refresh} size={14} />
                    道具を読み直す
                  </button>
                </Show>
              }
            >
              <BulkPermissions scope={scope()} />
              <div class="list">
                <For each={detail()?.tools ?? []}>
                  {(t) => (
                    <ToolRow
                      t={t}
                      scope={scope()}
                      owner={conn().name}
                      readOnlyTag={ours()}
                      source={
                        <Show when={t.source === KIND_SOURCE && ours()}>
                          <A class="sub" href={kindHref(conn().kind)}>
                            {t.source}
                          </A>
                        </Show>
                      }
                    />
                  )}
                </For>
                <Show when={ours() && (detail()?.tools ?? []).length === 0}>
                  <p class="empty">道具をまだ読んでいない。「道具を読み直す」を押す</p>
                </Show>
              </div>
            </Pref>
          </div>
        </>
      )}
    </Show>
  );
}
