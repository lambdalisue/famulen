import { Collapsible } from "@ark-ui/solid/collapsible";
import { createAsync, query } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import Icon from "~/components/Icon";
import GatewayLog from "~/components/logs/GatewayLog";
import LogSheet from "~/components/logs/LogSheet";
import { Permission } from "~/gen/famulen/v1/services_pb";
import type { Secret } from "~/gen/famulen/v1/vault_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { personaQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { shortTime } from "~/lib/time";
import { setView } from "~/lib/view";

const vaultQuery = query(() => api.vault.listSecrets({}), "vault");

/** Who wrote a secret, as the owner reads it. */
const WRITERS: Record<string, string> = {
  owner: "主人",
  secretary: "秘書",
  gateway: "ゲートウェイ",
};

/** The form's text field, trimmed. */
function text(f: FormData, name: string): string {
  return String(f.get(name) ?? "").trim();
}

/** What an automatically added secret belongs to. */
function holder(s: Secret): string {
  if (s.connection) return `つなぎ先「${s.connection}」のログイン・鍵`;
  if (s.model) return `モデル「${s.model}」の鍵`;
  return "";
}

/** When it was put in and, if it was changed since, who changed it and when. */
function written(s: Secret, tz: string): string {
  const writer = WRITERS[s.by] ?? s.by;
  if (s.updatedAt === s.createdAt) return `${writer}が ${shortTime(s.createdAt, tz)} に入れた`;
  return `${shortTime(s.createdAt, tz)} に入れた · ${writer}が ${shortTime(s.updatedAt, tz)} に直した`;
}

/** The secret's description, rewritten in place without touching its value. */
function Description(props: { s: Secret }) {
  const [editing, setEditing] = createSignal(false);
  const save = async (e: SubmitEvent) => {
    e.preventDefault();
    const form = e.currentTarget;
    if (!(form instanceof HTMLFormElement)) return;
    const description = text(new FormData(form), "description");
    if (description === props.s.description) {
      setEditing(false);
      return;
    }
    const done = await act(() => api.vault.describeSecret({ name: props.s.name, description }), "説明を直した");
    if (done) setEditing(false);
  };
  return (
    <Show
      when={editing()}
      fallback={
        <button
          type="button"
          class="vault-desc"
          aria-label={`${props.s.name}の説明を直す`}
          onClick={() => setEditing(true)}
        >
          <span class={props.s.description ? "" : "sub"}>{props.s.description || "説明なし"}</span>
          <Icon svg={icons.pencil} size={12} />
        </button>
      }
    >
      <form
        class="row vault-desc-form"
        onSubmit={save}
      >
        <input
          class="input grow"
          name="description"
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              setEditing(false);
            }
          }}
          value={props.s.description}
          placeholder="何に使うか"
          aria-label={`${props.s.name}の説明`}
          ref={(el) => queueMicrotask(() => el.focus())}
        />
        <button type="button" class="btn small" onClick={() => setEditing(false)}>
          やめる
        </button>
        <button class="btn small primary">直す</button>
      </form>
    </Show>
  );
}

function Row(props: { s: Secret; tz: string }) {
  const [mode, setMode] = createSignal<"view" | "replace" | "delete">("view");
  const replace = async (e: SubmitEvent) => {
    e.preventDefault();
    const form = e.currentTarget;
    if (!(form instanceof HTMLFormElement)) return;
    const value = String(new FormData(form).get("value") ?? "");
    form.reset();
    const done = await act(() => api.vault.setSecretValue({ name: props.s.name, value }), "置き換えた");
    if (done) setMode("view");
  };
  return (
    <div class="item vault-row">
      <div class="vault-main">
        <b class="mono vault-name">{props.s.name}</b>
        <Description s={props.s} />
        <span class="sub">
          <Show when={props.s.automatic}>{holder(props.s)} · </Show>
          {written(props.s, props.tz)}
        </span>
      </div>
      <Show when={mode() === "view"}>
        <div class="vault-actions">
          <button
            type="button"
            class="btn small"
            aria-label={`${props.s.name}を置き換える`}
            onClick={() => setMode("replace")}
          >
            <Icon svg={icons.refresh} size={12} />
          </button>
          <Show when={!props.s.automatic}>
            <button
              type="button"
              class="btn small danger"
              aria-label={`${props.s.name}を消す`}
              onClick={() => setMode("delete")}
            >
              <Icon svg={icons.trash} size={12} />
            </button>
          </Show>
        </div>
      </Show>
      <Show when={mode() === "delete"}>
        <div class="row vault-wide" role="group" aria-label={`${props.s.name}を消すか確かめる`}>
          <span class="sub grow">消すと、ゲートウェイも主人も、もう取り出せない。</span>
          <button type="button" class="btn small" onClick={() => setMode("view")}>
            やめる
          </button>
          <button
            type="button"
            class="btn small danger"
            onClick={() => act(() => api.vault.deleteSecret({ id: props.s.name }), "消した")}
          >
            消す
          </button>
        </div>
      </Show>
      <Show when={mode() === "replace"}>
        <form class="row vault-wide" onSubmit={replace}>
          <input
            class="input grow"
            name="value"
            type="password"
            autocomplete="new-password"
            required
            aria-label={`${props.s.name}の新しい中身`}
          />
          <button type="button" class="btn small" onClick={() => setMode("view")}>
            やめる
          </button>
          <button class="btn small primary">置き換える</button>
        </form>
      </Show>
    </div>
  );
}

const permissionsQuery = query(() => api.vault.getVaultPermissions({}), "vault-permissions");

/** What each permission means when the secretary changes the vault. */
const CHOICES: [Permission, string][] = [
  [Permission.AUTO, "まかせる"],
  [Permission.ALLOW, "尋ねない"],
  [Permission.CONFIRM, "尋ねる"],
  [Permission.FORBID, "拒否する"],
];

type Change = "put" | "describe" | "delete";

/** Whether the owner is asked before the secretary changes the vault. */
function SecretaryPermissions() {
  const permissions = createAsync(() => permissionsQuery());
  const choose = (change: Change, permission: Permission) =>
    act(() => api.vault.setVaultPermission({ change, permission }), "変えた");
  const line = (change: Change, label: string) => (
    <div class="row" role="group" aria-label={label} style={{ gap: "6px", "flex-wrap": "wrap" }}>
      <span class="grow">{label}</span>
      <For each={CHOICES}>
        {([permission, name]) => (
          <button
            type="button"
            class={`btn small${permissions()?.[change] === permission ? " primary" : ""}`}
            aria-pressed={permissions()?.[change] === permission}
            onClick={() => choose(change, permission)}
          >
            {name}
          </button>
        )}
      </For>
    </div>
  );
  return (
    <section class="card" style={{ display: "flex", "flex-direction": "column", gap: "8px" }}>
      <h3>秘書が金庫を変えるとき</h3>
      {line("put", "秘密を入れる・置き換える")}
      {line("describe", "秘密の説明を直す")}
      {line("delete", "秘密を消す")}
      <p class="sub">金庫を変えるのは取り返しがつかないので、「まかせる」でも尋ねる。</p>
      <p class="sub">尋ねているあいだ、秘書が入れようとした秘密は金庫に封じられ、確認には名前だけが出る。</p>
    </section>
  );
}

export default function Vault() {
  setView("設定 · 金庫");
  const vault = createAsync(() => vaultQuery());
  const persona = createAsync(() => personaQuery());
  const tz = () => persona()?.timezone ?? "UTC";
  const added = () => (vault()?.secrets ?? []).filter((s) => !s.automatic);
  const automatic = () => (vault()?.secrets ?? []).filter((s) => s.automatic);

  const add = async (e: SubmitEvent) => {
    e.preventDefault();
    const form = e.currentTarget;
    if (!(form instanceof HTMLFormElement)) return;
    const f = new FormData(form);
    const request = { name: text(f, "name"), purpose: text(f, "purpose"), value: String(f.get("value") ?? "") };
    const field = form.elements.namedItem("value");
    if (field instanceof HTMLInputElement) field.value = "";
    const done = await act(() => api.vault.setSecretValue(request), "金庫に入れた");
    if (done) form.reset();
  };

  return (
    <>
      <h1>金庫</h1>
      <p class="sub">
        パスワードやトークンのような秘密は、ここに入れる。中身を読めるのはゲートウェイだけで、外のサービスを呼ぶときにだけ使う。入れたあとは、主人にも秘書にも、中身はどこにも一文字も出ない。
      </p>
      <Show when={vault()?.unavailable}>
        <p class="card">
          <span class="tag warn">使えない</span> {vault()?.unavailable}
        </p>
      </Show>
      <Show when={vault() && !vault()?.unavailable}>
        <section class="vault-group" aria-label="足した秘密">
          <h2 class="vault-group-title">足した秘密</h2>
          <div class="list">
            <For each={added()}>{(s) => <Row s={s} tz={tz()} />}</For>
            <Show when={added().length === 0}>
              <p class="empty">まだない</p>
            </Show>
          </div>
          <form class="card row vault-add" onSubmit={add}>
            <input class="input mono" name="name" required placeholder="名前（bank-password）" aria-label="名前" />
            <input class="input grow" name="purpose" placeholder="何に使うか" aria-label="何に使うか" />
            <input
              class="input grow"
              name="value"
              type="password"
              autocomplete="new-password"
              required
              placeholder="中身"
              aria-label="中身"
            />
            <button class="btn primary">
              <Icon svg={icons.plus} size={14} />
              入れる
            </button>
          </form>
        </section>
        <Show when={automatic().length > 0}>
          <Collapsible.Root class="vault-group vault-auto">
            <Collapsible.Trigger class="vault-auto-trigger">
              <Icon svg={icons.chevronRight} size={14} />
              <span>自動で入った秘密</span>
              <span class="tag">{automatic().length}</span>
            </Collapsible.Trigger>
            <Collapsible.Content class="vault-auto-content">
              <p class="sub">
                ログインやつなぎ先の鍵、モデルの登録のときに、ゲートウェイが入れたもの。中身は置き換えられるが、消すときはつなぎ先かモデルを外す。
              </p>
              <div class="list">
                <For each={automatic()}>{(s) => <Row s={s} tz={tz()} />}</For>
              </div>
            </Collapsible.Content>
          </Collapsible.Root>
        </Show>
        <SecretaryPermissions />
      </Show>
      <LogSheet title="ゲートウェイの控え" label="控えを見る">
        <GatewayLog />
      </LogSheet>
    </>
  );
}
