import { createAsync, revalidate } from "@solidjs/router";
import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import AddModel from "~/components/AddModel";
import { type AccountLimits, AgentCli, type Model, type ModelLimits, ModelKind, Role, type RoleSetting } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { createLate } from "~/lib/kept";
import { modelLimitsQuery, modelsQuery, isMetered, kindLabel, limitView } from "~/lib/models";
import { useOwner } from "~/lib/owner";
import * as icons from "~/lib/icons";
import { setView } from "~/lib/view";
import { Button, EmptyState, Form, IconButton, List, PageHeader, Tag, TextField, whenConfirmed } from "~/components/ui";

/** The settings page where each role's model is chosen. */
const ROLE_PAGES: Record<Role, string> = {
  [Role.UNSPECIFIED]: "/settings",
  [Role.SECRETARY]: "/settings",
  [Role.DREAM]: "/settings/roles/dream",
  [Role.LOOKOUT]: "/settings/lookout",
  [Role.TRIAGE]: "/settings/roles/triage",
  [Role.ASSESS]: "/settings/roles/assess",
  [Role.WORKER]: "/settings/roles/worker",
  [Role.VOICE]: "/settings/roles/voice",
};

/** The group of models found nowhere, whose accounts are told by their program. */
const OWN = "自分で足したもの";

/** The list, parted by where the models come from. */
const GROUPS: [string, (m: Model) => boolean][] = [
  ["Claude Code", (m) => m.found && m.cli === AgentCli.CLAUDE_CODE],
  ["Codex", (m) => m.found && m.cli === AgentCli.CODEX],
  ["このマシンの Ollama", (m) => m.found && m.kind === ModelKind.LOCAL],
  [OWN, (m) => !m.found],
];

export default function Models() {
  setView("設定", "登録したモデル");
  const models = createAsync(() => modelsQuery());

  const [finding, setFinding] = createSignal(false);
  const find = async () => {
    setFinding(true);
    await act(() => api.models.findModels({}), "探し直した");
    setFinding(false);
  };
  // Read apart from the list and late, as the agent CLIs take seconds to tell.
  const read = createLate(() => modelLimitsQuery());
  // A reading that failed leaves what was read before.
  const limits = createMemo<ModelLimits | undefined>((before) => read() ?? before);
  const { tz } = useOwner();
  const [now, setNow] = createSignal(Date.now());
  onMount(() => {
    const tick = setInterval(() => setNow(Date.now()), 60_000);
    // Just past the five minutes the backend keeps an answer, so each reading is a new one.
    const reread = setInterval(() => void revalidate(modelLimitsQuery.key), 5.5 * 60_000);
    onCleanup(() => {
      clearInterval(tick);
      clearInterval(reread);
    });
  });
  const groups = () =>
    GROUPS.map(([label, holds]) => ({ label, models: (models()?.models ?? []).filter(holds) })).filter((g) => g.models.length > 0);
  // Apart from the groups, so that limits read again leave the rows, and a key being typed in one, alone.
  const placed = createMemo(() => {
    const by = new Map<string, AccountLimits[]>();
    const shown = new Set<AccountLimits>();
    for (const g of groups()) {
      const ids = new Set(g.models.map((m) => m.id));
      // An account goes with the first group that has one of its models.
      const accounts = (limits()?.accounts ?? []).filter((a) => !shown.has(a) && a.models.some((id) => ids.has(id)));
      for (const a of accounts) shown.add(a);
      by.set(g.label, accounts);
    }
    return by;
  });

  return (
    <>
      <PageHeader
        title="登録したモデル"
        note="エージェント（秘書・夢・見張り・トリアージ・見極め）が使えるモデル。このマシンで使えるものは自動で入る。どのエージェントにどれを使うかは、それぞれのエージェントの頁で選ぶ。"
        actions={<AddModel candidates={models()?.candidates ?? []} />}
      />
      <section class="model-groups">
        <div class="model-groups-head">
          <h2>一覧</h2>
          <Button small icon={icons.refresh} disabled={finding()} onClick={() => find()}>
            {finding() ? "探している" : "探し直す"}
          </Button>
        </div>
        <For each={groups()} fallback={<EmptyState>まだない</EmptyState>}>
          {(g) => (
            <div class="model-group">
              <h3>{g.label}</h3>
              <For each={placed().get(g.label) ?? []}>
                {(a) => <LimitGauges account={a} named={g.label === OWN} now={now()} tz={tz()} />}
              </For>
              <List>
                <For each={g.models}>{(m) => <ModelRow model={m} roles={models()?.roles ?? []} />}</For>
              </List>
            </div>
          )}
        </For>
      </section>
    </>
  );
}

/** How much of one account's limits is used, and when each starts again. */
function LimitGauges(props: { account: AccountLimits; named: boolean; now: number; tz: string }) {
  const title = () => `${props.named ? `${props.account.endpoint} の` : ""}アカウントの利用上限`;
  return (
    <div class="model-limits" role="group" aria-label={title()}>
      <span class="sub model-limits-head">{title()}</span>
      <For each={props.account.limits}>
        {(limit) => {
          const v = () => limitView(limit, props.now, props.tz);
          return (
            <div class="model-limit" data-tone={v().tone}>
              <div class="meter-row">
                <span>{v().label}</span>
                <span class="mono">{v().usedText}</span>
                <div class="meter" role="meter" aria-label={v().label} aria-valuenow={v().used} aria-valuemin={0} aria-valuemax={100}>
                  <div style={{ width: `${v().used}%` }} />
                </div>
              </div>
              <Show when={v().note}>
                <span class="sub">{v().note}</span>
              </Show>
            </div>
          );
        }}
      </For>
    </div>
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
    <div class="item wraps" role="listitem">
      <Show when={!m().found}>
        <Tag>{kindLabel(m().kind)}</Tag>
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
                  <a href={ROLE_PAGES[r.role]}>{r.label}</a>
                </>
              )}
            </For>
          </span>
        </Show>
      </div>
      <span class="sub">{price()}</span>
      <Show when={PRICED.includes(m().kind)}>
        <IconButton icon={icons.pencil} label={`${m().name}の鍵を入れ替える`} onClick={() => setReplacing(true)} />
      </Show>
      <IconButton
        icon={icons.trash}
        label={`${m().name}を外す`}
        variant="danger"
        disabled={users().length > 0}
        title={users().length > 0 ? "使っているエージェントがあるので外せない" : "外す"}
        onClick={() =>
          whenConfirmed(
            `モデル「${m().name}」を外す`,
            "外す",
            () => act(() => api.models.unregisterModel({ id: m().id }), "外した"),
            m().found ? "「モデルの追加」で戻せる。" : "鍵も金庫から消える。",
          )
        }
      />
      <Show when={replacing()}>
        <Form class="row item-more" onSubmit={replace}>
          <TextField class="grow" name="api_key" type="password" autocomplete="new-password" required aria-label="新しい API キー" />
          <Button small onClick={() => setReplacing(false)}>
            やめる
          </Button>
          <Button variant="primary" small type="submit">
            入れ替える
          </Button>
        </Form>
      </Show>
    </div>
  );
}

const PRICED = [ModelKind.API, ModelKind.JUDGE, ModelKind.TRANSCRIBE];
