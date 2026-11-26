import { shortTime } from "~/lib/dates";
import { createAsync, useNavigate, useSearchParams } from "@solidjs/router";
import { createEffect, createSignal, For, type JSX, Show } from "solid-js";
import AddService from "~/components/AddService";
import AskAbout from "~/components/AskAbout";
import DefinitionDiff from "~/components/DefinitionDiff";
import PermissionSeg, { PERMISSIONS } from "~/components/PermissionSeg";
import { CalendarsSection, ChannelsSection, LookoutSection } from "~/components/ServiceSections";
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
import { byName, hasWords } from "~/lib/words";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { setView } from "~/lib/view";
import {
  Button,
  Callout,
  Disclosure,
  EmptyState,
  Field,
  Form,
  Keyed,
  List,
  ListRow,
  Loading,
  PageHeader,
  SearchBox,
  Section,
  Select,
  Tag,
  TextField,
  whenConfirmed,
} from "~/components/ui";

const OWN = "famulen";
const GOOGLE = "google";
const SLACK = "slack";

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
  const sub = () =>
    [props.c.account && props.c.account !== props.c.name ? props.c.account : "", props.c.watching].filter(Boolean).join(" · ");
  return (
    <ListRow
      leads
      icon={props.icon}
      href={`/settings/services?id=${encodeURIComponent(props.c.id)}`}
      title={props.c.name}
      sub={
        <>
          <Show when={sub()}>
            <span>{sub()}</span>
          </Show>
          <Show when={props.c.state !== ConnectionState.CONNECTED}>
            <span class="warn-text">{props.c.status}</span>
          </Show>
        </>
      }
      end={
        <Show
          when={props.c.state === ConnectionState.CONNECTED}
          fallback={<Tag tone="warn">{props.c.state === ConnectionState.LOGIN_PENDING ? "ログイン待ち" : "使えない"}</Tag>}
        >
          <Tag tone="ok">{props.c.status}</Tag>
        </Show>
      }
    />
  );
}

async function goLogin(run: () => Promise<{ loginUrl: string } | undefined>) {
  const r = await run();
  if (r?.loginUrl) location.href = r.loginUrl;
}

function ServiceList() {
  setView("設定", "サービス");
  const list = createAsync(() => connectionsQuery());
  const known = createAsync(() => knownQuery());
  const owner = useOwner();
  const [search, setSearch] = createSignal("");
  const connections = () => list()?.connections ?? [];
  const found = () => {
    return connections()
      .filter((c) => hasWords(`${c.name} ${c.kind} ${c.account}`, search()))
      .sort((a, b) => byName(a.name, b.name) || byName(a.account, b.account));
  };
  const iconOf = (kind: string) => iconNamed(known()?.find((k) => k.id === kind)?.icon ?? `brand-${kind}`);
  const nameOf = (kind: string) => known()?.find((k) => k.id === kind)?.name ?? kind;
  const kinds = () =>
    [
      ...new Set(
        connections()
          .map((c) => c.kind)
          .filter((k) => k),
      ),
    ].sort();

  return (
    <>
      <PageHeader
        title="サービス"
        note="あなたのアカウントや端末をつなぐ。つないだものは秘書が使い、カレンダー・見張り・チャネルにも入る。同じサービスの別のアカウントも、いくつでも足せる。"
        actions={<AddService />}
      />
      <section class="stack">
        <h2>つないだサービス</h2>
        <Show when={connections().length > 0} fallback={<EmptyState>まだ何もつないでいない</EmptyState>}>
          <SearchBox
            label="つないだサービスを探す"
            placeholder="名前・アカウントで探す"
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
          />
          <List>
            <For each={found()} fallback={<EmptyState>「{search()}」に当たるものはない</EmptyState>}>
              {(c) => <ConnectionRow c={c} icon={iconOf(c.kind)} />}
            </For>
          </List>
        </Show>
      </section>
      <Show when={kinds().length > 0}>
        <section class="stack">
          <h2>道具の権限の既定</h2>
          <p class="sub">
            道具を秘書にどこまで使わせるかを、サービスごとにまとめて決める。同じサービスのどのアカウントにも効き、アカウントごとに上書きもできる。
          </p>
          <List>
            <For each={kinds()}>
              {(k) => (
                <ListRow
                  leads
                  icon={iconOf(k)}
                  href={kindHref(k)}
                  title={nameOf(k)}
                  sub={`${connections()
                    .filter((c) => c.kind === k)
                    .map((c) => c.account || c.name)
                    .join("、")} に効く`}
                />
              )}
            </For>
          </List>
        </section>
      </Show>
      <section class="stack">
        <h2>そのほか</h2>
        <List>
          <ListRow
            leads
            icon={icons.shieldCheck}
            href={`/settings/services?id=${OWN}`}
            title={`${owner.aide()} 自身の道具`}
            sub="会話・記録・記憶・予定・Slack"
          />
          <ListRow
            leads
            icon={icons.terminal}
            href="/settings/services?builtin=1"
            title="秘書の組み込みの道具"
            sub="ファイルを読む・書く・探す、シェル"
          />
        </List>
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
  const owner = useOwner();
  const tz = owner.tz;
  const byDefinition = () => {
    const groups = new Map<string, DefinitionChange[]>();
    for (const c of changes() ?? []) groups.set(c.definition, [...(groups.get(c.definition) ?? []), c]);
    return [...groups.entries()];
  };
  const revert = (id: string) => act(() => api.connections.revertDefinition({ id }), "一つ前に戻した");
  return (
    <Show when={byDefinition().length > 0}>
      <section class="stack">
        <h2>定義の変更</h2>
        <p class="sub">許して入ったサービス定義の変更。一番新しい変更は、一つ前に戻せる。戻したことも変更として残る。</p>
        <div class="prefs">
          <For each={byDefinition()}>
            {([definition, list]) => (
              <div class="pref" data-testid="definition-changes">
                <div class="row">
                  <b class="grow mono">{definition}</b>
                  <Button small icon={icons.arrowBackUp} aria-label={`${definition}を一つ前に戻す`} onClick={() => revert(definition)}>
                    一つ前に戻す
                  </Button>
                </div>
                <For each={list}>
                  {(c) => (
                    <Disclosure
                      class="change"
                      title={
                        <>
                          <span class="mono sub">{shortTime(c.at, tz())}</span>
                          <Tag>{CHANGE_KIND[c.kind] ?? c.kind}</Tag>
                          <span class="sub">{c.actor}</span>
                          <span class="grow">{c.why}</span>
                        </>
                      }
                    >
                      <DefinitionDiff before={c.before} after={c.after} />
                    </Disclosure>
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
    props.detail.receivers.filter((r) => !(channels() ?? []).some((c) => c.connection === props.id && c.receiver === r.name));
  return (
    <Show when={shown().length > 0}>
      <div class="pref-sub">
        <h3>受け口</h3>
        <p class="sub">外から届いたものを、手を加えずにどこへ渡すか。</p>
        <List>
          <For each={shown()}>
            {(r) => (
              <div role="listitem" class="item">
                <div class="grow">
                  <span class="mono">{r.name}</span> <span class="sub">{r.how}</span>
                </div>
                <label class="sub" for={`rx-${r.name}`}>
                  渡す先
                </label>
                <Select
                  id={`rx-${r.name}`}
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
                </Select>
              </div>
            )}
          </For>
        </List>
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
        <List>
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
                <Form class="item" onSubmit={save}>
                  <label class="grow" for={`av-${v.name}`}>
                    {v.label}
                    <Show when={v.secret}>
                      {" "}
                      <Tag>金庫</Tag>
                    </Show>
                  </label>
                  <TextField
                    id={`av-${v.name}`}
                    type={v.secret ? "password" : "text"}
                    placeholder={v.secret && v.set ? "入っている" : ""}
                    value={text()}
                    onInput={(e) => setText(e.currentTarget.value)}
                  />
                  <Button small type="submit">
                    {v.secret && v.set ? "入れ直す" : "保存"}
                  </Button>
                </Form>
              );
            }}
          </For>
          <For each={found()}>
            {(v) => (
              <div role="listitem" class="item">
                <span class="grow mono">{v.name}</span>
                <span class="sub mono">{v.value}</span>
              </div>
            )}
          </For>
        </List>
      </div>
    </Show>
  );
}

const BULK: [string, string][] = [
  ["read", "読むだけの道具"],
  ["write", "書く・送る道具"],
];
const BULK_RULES: [Permission, string][] = [...PERMISSIONS, [Permission.UNSPECIFIED, "既定に戻す"]];

function BulkPermissions(props: { scope: string }) {
  return (
    <div class="bulk">
      <span class="sub">まとめて選ぶ（押すとすぐ変わる）</span>
      <span class="sub">
        「まかせる」は、読むだけの道具はそのまま使い、消す・外の人へ送るなど取り返しのつかないものや人を巻き込むものは必ずあなたに確認し、それ以外は
        <a href="/settings/roles/assess">見極め</a>
        が頼まれたとおりかを判断する。「許可する」は尋ねずに使い、「確認する」は毎回あなたに尋ね、「拒否する」は使わせない。
      </span>
      <For each={BULK}>
        {([which, label]) => (
          <div class="row" role="group" aria-label={`${label}をまとめて選ぶ`}>
            <span class="bulk-which">{label}を</span>
            <For each={BULK_RULES}>
              {([permission, l]) => (
                <Button small onClick={() => act(() => api.connections.setPermissions({ scope: props.scope, which, permission }))}>
                  {l}
                </Button>
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
    <ListRow
      class="tool-row"
      title={<span class="mono">{props.t.name}</span>}
      sub={
        <>
          {props.source}
          <Show when={props.t.readOnly && props.readOnlyTag}>
            <Tag tone="ok">読むだけ</Tag>
          </Show>
          <Show when={props.t.alwaysAsk && props.t.effective === Permission.AUTO}>
            <Tag tone="warn" title="取り返しがつかないか、人を巻き込むので、まかせても必ず確認する">
              必ず確認する
            </Tag>
          </Show>
          <Show when={props.t.description}>
            <span>{[...props.t.description].slice(0, 120).join("")}</span>
          </Show>
        </>
      }
      end={
        <>
          <PermissionSeg
            tool={props.t.name}
            permission={props.t.permission}
            inherited={props.t.inherited}
            onChoose={(permission, done) =>
              void act(() => api.connections.setPermission({ scope: props.scope, tool: props.t.name, permission }), done)
            }
          />
          <AskAbout what={`${props.owner} の道具 ${props.t.name}`} />
        </>
      }
    />
  );
}

/** The secretary's model CLI's built-in tools and the owner's permission for each. */
function BuiltinTools() {
  const tools = createAsync(() => builtinToolsQuery());
  setView("設定", "サービス", "組み込みの道具");
  return (
    <>
      <PageHeader
        back={{ href: "/settings/services", label: "一覧へ" }}
        title={<>秘書の組み込みの道具</>}
        note="秘書のモデル CLI がはじめから持つ道具。どれを選んでも、ゲートウェイの置き場所と .env には届かない。"
      />
      <BulkPermissions scope="" />
      <List>
        <Keyed each={tools() ?? []} by={(t) => t.name}>
          {(t) => <ToolRow t={t()} scope="" owner="秘書の組み込み" readOnlyTag />}
        </Keyed>
        <Show when={tools() && (tools() ?? []).length === 0}>
          <EmptyState>秘書がまだ動いていないので、道具の一覧がない</EmptyState>
        </Show>
      </List>
    </>
  );
}

function KindDetail(props: { kind: string }) {
  const detail = createAsync(() => kindQuery(props.kind));
  const scope = () => `kind:${props.kind}`;
  setView("設定", "サービス種", props.kind);
  return (
    <>
      <PageHeader
        back={{ href: "/settings/services", label: "一覧へ" }}
        title={<>サービス種「{props.kind}」の既定</>}
        note="同じ種類のつなぎ先すべてに効く。つなぎ先ごとの決まりがあれば、そちらが勝つ。"
      />
      <BulkPermissions scope={scope()} />
      <div class="row">
        <span class="sub">この既定を使っているつなぎ先：</span>
        <For each={detail()?.connections ?? []}>
          {(c) => (
            <a class="tag" href={`/settings/services?id=${encodeURIComponent(c.id)}`}>
              {c.account ? `${c.name} · ${c.account}` : c.name}
            </a>
          )}
        </For>
      </div>
      <List>
        <Keyed each={detail()?.tools ?? []} by={(t) => t.name}>
          {(t) => <ToolRow t={t()} scope={scope()} owner={`サービス種「${props.kind}」`} readOnlyTag />}
        </Keyed>
        <Show when={detail() && (detail()?.tools ?? []).length === 0}>
          <EmptyState>この種類のつなぎ先は、まだ道具を読んでいない</EmptyState>
        </Show>
      </List>
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
  const owner = useOwner();

  createEffect(() => setView("設定", "サービス", { told: props.id, shown: c()?.name ?? props.id }));

  const edit = (field: "name" | "kind" | "account", value: string) =>
    act(() =>
      api.connections.editConnection(
        field === "name"
          ? { id: props.id, name: value }
          : field === "kind"
            ? { id: props.id, kind: value }
            : { id: props.id, account: value },
      ),
    );

  const setSecret = async (e: SubmitEvent) => {
    e.preventDefault();
    const done = await act(() => api.connections.setSecret({ id: props.id, apiKey: key(), header: header() }), "鍵を入れた");
    if (done) setKey("");
  };

  const login = () =>
    goLogin(async () => {
      const go = await act(() => api.connections.startLogin({ id: props.id, consoleOrigin: location.origin }), "");
      return go && { loginUrl: go.url };
    });

  return (
    <Show when={c()} fallback={<Loading />}>
      {(conn) => (
        <>
          <PageHeader
            back={{ href: "/settings/services", label: "一覧へ" }}
            title={conn().name}
            tags={<Tag>{conn().transport}</Tag>}
            note={
              own()
                ? "会話・記録・記憶・予定・Slack への投稿など、Famulen が自分で持つ道具。ほかのつなぎ先と同じように任せ方を決められる。"
                : conn().account
            }
          />
          <Show when={ours()}>
            <Show when={conn().state === ConnectionState.CONNECTED} fallback={<Callout tone="warn">{conn().status}</Callout>}>
              <Callout icon={icons.check}>{conn().status}</Callout>
            </Show>
          </Show>
          <div class="prefs">
            <Show when={ours()}>
              <Section
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
                    <Field label="名前" hint="一覧や見張りでの呼び名。好きに変えてよく、つながりは変わらない。">
                      <TextField value={conn().name} required onChange={(e) => edit("name", e.currentTarget.value)} />
                    </Field>
                    <Show
                      when={defined()}
                      fallback={
                        <Field
                          label="アカウントの目印"
                          hint="誰のアカウントかを見分けるための書き置き。サービスから分かれば初めから入る。書き換えても、つながるアカウントは変わらない。"
                        >
                          <TextField value={conn().account} onChange={(e) => edit("account", e.currentTarget.value)} />
                        </Field>
                      }
                    >
                      <Field
                        label="アカウント"
                        hint="つないだアカウント。ここでは変えられない。別のアカウントを使うなら、サービスを足す。"
                        group
                      >
                        <div>{conn().account}</div>
                      </Field>
                    </Show>
                  </div>
                </Show>
                <Show when={detail()?.login && conn().state === ConnectionState.CONNECTED}>
                  <p class="sub">つながっているので、ログインは要らない。切れたときや、許す範囲を変えるときにだけログインし直す。</p>
                </Show>
                <div class="row">
                  <Show when={detail()?.login}>
                    <Button icon={icons.login2} classList={{ primary: conn().state !== ConnectionState.CONNECTED }} onClick={login}>
                      {conn().state === ConnectionState.LOGIN_PENDING ? "ログインを続ける" : "ログインし直す"}
                    </Button>
                  </Show>
                  <Button
                    variant="danger"
                    icon={icons.trash}
                    onClick={() =>
                      whenConfirmed(
                        `「${conn().name}」を外す`,
                        "外す",
                        async () => {
                          // Leave first: reloading the page of a removed connection would fail.
                          navigate("/settings/services");
                          await act(() => api.connections.deleteConnection({ id: props.id }), "外した");
                        },
                        "このつなぎ先の道具・チャネル・見張りがすべてなくなり、鍵も金庫から消える。",
                      )
                    }
                  >
                    外す
                  </Button>
                </div>
                <Show when={!managed() && conn().transportKind === TransportKind.API}>
                  <Form class="row" onSubmit={setSecret}>
                    <TextField
                      placeholder="Authorization"
                      aria-label="鍵を入れるヘッダー"
                      value={header()}
                      onInput={(e) => setHeader(e.currentTarget.value)}
                    />
                    <TextField
                      class="grow"
                      type="password"
                      placeholder={detail()?.hasSecret ? "金庫の API キーを置き換える" : "API キー"}
                      aria-label="API キー"
                      value={key()}
                      onInput={(e) => setKey(e.currentTarget.value)}
                    />
                    <Button small type="submit">
                      鍵を入れる
                    </Button>
                  </Form>
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
                  <Disclosure title="詳しい設定">
                    <div class="connection-fields">
                      <Show when={detail()?.target}>
                        <Field label={conn().transport} group>
                          <div class="mono service-target">{detail()?.target}</div>
                        </Field>
                      </Show>
                      <Field
                        label="サービス種"
                        hint="同じサービス種のつなぎ先は、道具の権限の既定を分け合う。変えると、新しい種類の既定に従う。ふつうは変えない。"
                      >
                        <TextField class="mono" value={conn().kind} required onChange={(e) => edit("kind", e.currentTarget.value)} />
                      </Field>
                    </div>
                  </Disclosure>
                </Show>
              </Section>
              <ChannelsSection connection={props.id} />
              <Show when={conn().kind === GOOGLE}>
                <CalendarsSection connection={props.id} account={conn().account || conn().name} timezone={owner.tz()} />
              </Show>
              <LookoutSection connection={props.id} />
            </Show>
            <Section
              title="道具"
              note={
                <Show when={ours()} fallback="このつなぎ先の道具を、秘書にどこまで使わせるか。">
                  いまある道具と、どこまで使わせるか。道具ごとに、サービス種「{conn().kind}」の既定を使うか上書きするかを選ぶ。
                  <Show when={conn().kind}>
                    {" "}
                    <a href={kindHref(conn().kind)}>サービス種「{conn().kind}」の既定を見る</a>
                  </Show>
                </Show>
              }
              action={
                <Show when={ours() && !managed()}>
                  <Button
                    small
                    icon={icons.refresh}
                    onClick={() => act(() => api.connections.refreshTools({ id: props.id }), "道具を読み直した")}
                  >
                    道具を読み直す
                  </Button>
                </Show>
              }
            >
              <BulkPermissions scope={scope()} />
              <List>
                <Keyed each={detail()?.tools ?? []} by={(t) => t.name}>
                  {(t) => (
                    <ToolRow
                      t={t()}
                      scope={scope()}
                      owner={conn().name}
                      readOnlyTag={ours()}
                      source={
                        <Show when={t().source === KIND_SOURCE && ours()}>
                          <a class="sub" href={kindHref(conn().kind)}>
                            {t().source}
                          </a>
                        </Show>
                      }
                    />
                  )}
                </Keyed>
                <Show when={ours() && (detail()?.tools ?? []).length === 0}>
                  <EmptyState>道具をまだ読んでいない。「道具を読み直す」を押す</EmptyState>
                </Show>
              </List>
            </Section>
          </div>
        </>
      )}
    </Show>
  );
}
