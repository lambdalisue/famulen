import { A, createAsync } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import AddModel from "~/components/AddModel";
import Icon from "~/components/Icon";
import { AgentCli, type Model, ModelKind, Role, type RoleSetting } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { modelsQuery, isMetered, KINDS, kindLabel } from "~/lib/models";
import * as icons from "~/lib/icons";
import { setView } from "~/lib/view";

/** The settings page where each role's model is chosen. */
const ROLE_PAGES: Record<Role, string> = {
  [Role.UNSPECIFIED]: "/settings",
  [Role.SECRETARY]: "/settings",
  [Role.DREAM]: "/settings/roles/dream",
  [Role.LOOKOUT]: "/settings/lookout",
  [Role.TRIAGE]: "/settings/roles/triage",
  [Role.ASSESS]: "/settings/roles/assess",
};

/** The list, parted by where the models come from. */
const GROUPS: [string, (m: Model) => boolean][] = [
  ["Claude Code", (m) => m.found && m.cli === AgentCli.CLAUDE_CODE],
  ["Codex", (m) => m.found && m.cli === AgentCli.CODEX],
  ["このマシンの Ollama", (m) => m.found && m.kind === ModelKind.LOCAL],
  ["自分で足したもの", (m) => !m.found],
];

export default function Models() {
  setView("設定 · 登録したモデル");
  const models = createAsync(() => modelsQuery());

  const [finding, setFinding] = createSignal(false);
  const find = async () => {
    setFinding(true);
    await act(() => api.models.findModels({}), "探し直した");
    setFinding(false);
  };
  const groups = () =>
    GROUPS.map(([label, holds]) => ({ label, models: (models()?.models ?? []).filter(holds) })).filter(
      (g) => g.models.length > 0,
    );

  return (
    <>
      <div class="page-head">
        <div>
          <h1>登録したモデル</h1>
          <p class="sub">
            エージェント（秘書・夢・見張り・トリアージ・見極め）が使えるモデル。このマシンで使えるものは自動で入る。どのエージェントにどれを使うかは、それぞれのエージェントの頁で選ぶ。
          </p>
        </div>
        <AddModel candidates={models()?.candidates ?? []} />
      </div>
      <section class="model-groups">
        <div class="model-groups-head">
          <h2>一覧</h2>
          <button type="button" class="btn small" disabled={finding()} onClick={() => void find()}>
            <Icon svg={icons.refresh} size={14} />
            {finding() ? "探している" : "探し直す"}
          </button>
        </div>
        <For each={groups()} fallback={<p class="empty">まだない</p>}>
          {(g) => (
            <div class="model-group">
              <h3>{g.label}</h3>
              <div class="list">
                <For each={g.models}>{(m) => <ModelRow model={m} roles={models()?.roles ?? []} />}</For>
              </div>
            </div>
          )}
        </For>
      </section>
    </>
  );
}

function ModelRow(props: { model: Model; roles: RoleSetting[] }) {
  const m = () => props.model;
  const users = () => props.roles.filter((r) => r.model === m().id);
  const price = () => {
    if (isMetered(m())) return "従量";
    if (m().monthlyYen > 0) return `月 ${m().monthlyYen} 円`;
    return "無料・定額";
  };
  const [replacing, setReplacing] = createSignal(false);
  const replace = async (e: SubmitEvent) => {
    e.preventDefault();
    const form = e.currentTarget;
    if (!(form instanceof HTMLFormElement)) return;
    const apiKey = String(new FormData(form).get("api_key") ?? "");
    form.reset();
    const done = await act(() => api.models.setModelKey({ id: m().id, apiKey }), "入れ替えた");
    if (done) setReplacing(false);
  };
  return (
    <div class="item" style={{ "flex-wrap": "wrap" }}>
      <Show when={!m().found}>
        <span class="tag">{kindLabel(m().kind)}</span>
      </Show>
      <div class="grow">
        <b>{m().name}</b>
        <br />
        <span class="sub mono">
          {m().model}
          {m().endpoint && !m().found ? ` · ${m().endpoint}` : ""}
        </span>
        <Show when={m().hasKey}>
          <br />
          <span class="sub">API キーは金庫にある</span>
        </Show>
        <Show when={users().length > 0}>
          <br />
          <span class="sub">
            使うエージェント：
            <For each={users()}>
              {(r, i) => (
                <>
                  {i() > 0 ? "・" : ""}
                  <A href={ROLE_PAGES[r.role]}>{r.label}</A>
                </>
              )}
            </For>
          </span>
        </Show>
      </div>
      <span class="sub">{price()}</span>
      <Show when={PRICED.includes(m().kind)}>
        <button
          type="button"
          class="btn small"
          aria-label="鍵を入れ替える"
          title="鍵を入れ替える"
          onClick={() => setReplacing(true)}
        >
          <Icon svg={icons.pencil} size={12} />
        </button>
      </Show>
      <button
        type="button"
        class="btn small danger"
        disabled={users().length > 0}
        aria-label="外す"
        title={users().length > 0 ? "使っているエージェントがあるので外せない" : "外す"}
        onClick={() => void act(() => api.models.unregisterModel({ id: m().id }), "外した")}
      >
        <Icon svg={icons.trash} size={12} />
      </button>
      <Show when={replacing()}>
        <form class="row" style={{ width: "100%" }} onSubmit={replace}>
          <input
            class="input grow"
            name="api_key"
            type="password"
            autocomplete="new-password"
            required
            data-1p-ignore
            data-lpignore="true"
            data-bwignore
            aria-label="新しい API キー"
          />
          <button type="button" class="btn small" onClick={() => setReplacing(false)}>
            やめる
          </button>
          <button class="btn small primary">入れ替える</button>
        </form>
      </Show>
    </div>
  );
}

const PRICED = [ModelKind.API, ModelKind.JUDGE, ModelKind.TRANSCRIBE];
