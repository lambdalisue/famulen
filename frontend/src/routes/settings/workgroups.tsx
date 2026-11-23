import { Dialog } from "@ark-ui/solid/dialog";
import { RadioGroup } from "@ark-ui/solid/radio-group";
import { SegmentGroup } from "@ark-ui/solid/segment-group";
import { Select, createListCollection } from "@ark-ui/solid/select";
import { Switch } from "@ark-ui/solid/switch";
import { createAsync, query, useSearchParams } from "@solidjs/router";
import { For, Show, createSignal } from "solid-js";
import { Portal } from "solid-js/web";
import Icon from "~/components/Icon";
import Modal from "~/components/Modal";
import ZoneSection from "~/components/ZoneSection";
import { CalendarMode } from "~/gen/famulen/v1/calendar_pb";
import { type AccountSetting, type Member, MemberKind } from "~/gen/famulen/v1/workgroups_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { personaQuery, workgroupsQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { setView } from "~/lib/view";

const COLORS: [string, string][] = [
  ["青", "#2a78d6"],
  ["橙", "#E0622F"],
  ["青緑", "#0F6E68"],
  ["紫", "#6250d6"],
  ["緑", "#008300"],
];

const MODES: [CalendarMode, string, string][] = [
  [CalendarMode.SHOW, "show", "出す"],
  [CalendarMode.FAINT, "faint", "薄く出す"],
  [CalendarMode.HIDE, "hide", "出さない"],
];

/** What belonging to a workgroup means, for each kind of member. */
const KINDS: [MemberKind, string, string][] = [
  [MemberKind.CHANNEL, "話しかけてくる場所", "ここから来た頼みは、このワークグループのこととして受ける。"],
  [
    MemberKind.CONNECTION,
    "確かめずに使うサービス",
    "入れたサービスの道具は、このワークグループではいつもどおり使う。入れていないものは、使う前に毎回あなたに確かめる。",
  ],
  [MemberKind.LOOKOUT, "見張るもの", "見張りが拾ったものを、このワークグループのこととして扱う。"],
];

const NONE = "none";

/** The built-in workgroup that holds everything, shown as 全体 here. */
const WHOLE = "all";

const workgroupQuery = query((id: string) => api.workgroups.getWorkgroup({ id }), "workgroup");

const sameColor = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** The name and colour of a workgroup, as the add and edit dialogs ask them. */
function WorkgroupForm(props: {
  name: string;
  color: string;
  submit: string;
  onSubmit: (name: string, color: string) => void;
}) {
  const [name, setName] = createSignal(props.name);
  const [color, setColor] = createSignal(props.color);
  return (
    <form
      class="dialog-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (name().trim()) props.onSubmit(name().trim(), color());
      }}
    >
      <label class="dialog-field">
        <span>名前</span>
        <input
          class="input"
          name="name"
          autocomplete="off"
          data-1p-ignore
          data-lpignore="true"
          data-bwignore
          placeholder="趣味"
          required
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
      </label>
      <RadioGroup.Root
        class="dialog-field"
        value={COLORS.find(([, c]) => sameColor(c, color()))?.[1] ?? COLORS[0][1]}
        onValueChange={(d) => d.value && setColor(d.value)}
      >
        <RadioGroup.Label>色</RadioGroup.Label>
        <div class="swatches">
          <For each={COLORS}>
            {([label, c]) => (
              <RadioGroup.Item value={c} class="swatch" aria-label={label}>
                <RadioGroup.ItemControl class="swatch-dot" style={{ background: c }} />
                <RadioGroup.ItemText class="swatch-label">{label}</RadioGroup.ItemText>
                <RadioGroup.ItemHiddenInput />
              </RadioGroup.Item>
            )}
          </For>
        </div>
      </RadioGroup.Root>
      <div class="dialog-actions">
        <Dialog.CloseTrigger class="btn" type="button">
          やめる
        </Dialog.CloseTrigger>
        <button class="btn primary">{props.submit}</button>
      </div>
    </form>
  );
}

function AccountRow(props: { workgroup: string; account: AccountSetting }) {
  const collection = () =>
    createListCollection({
      items: [{ label: "決めない", value: NONE }, ...props.account.choices.map((c) => ({ label: c.label, value: c.id }))],
    });
  return (
    <div class="wg-row">
      <span class="grow">{props.account.label}</span>
      <Select.Root
        class="wg-select"
        collection={collection()}
        value={[props.account.connection || NONE]}
        onValueChange={(d) => {
          const v = d.value[0] ?? NONE;
          void act(() =>
            api.workgroups.setAccount({
              workgroup: props.workgroup,
              purpose: props.account.purpose,
              connection: v === NONE ? "" : v,
            }),
          );
        }}
        positioning={{ sameWidth: true }}
      >
        <Select.Control>
          <Select.Trigger class="wg-select-trigger" aria-label={props.account.label}>
            <Select.ValueText placeholder="決めない" />
            <Icon svg={icons.chevronDown} size={14} />
          </Select.Trigger>
        </Select.Control>
        <Portal>
          <Select.Positioner>
            <Select.Content class="menu-list">
              <For each={collection().items}>
                {(item) => (
                  <Select.Item item={item} class="menu-item">
                    <Select.ItemText>{item.label}</Select.ItemText>
                    <Select.ItemIndicator>
                      <Icon svg={icons.check} size={14} />
                    </Select.ItemIndicator>
                  </Select.Item>
                )}
              </For>
            </Select.Content>
          </Select.Positioner>
        </Portal>
        <Select.HiddenSelect />
      </Select.Root>
    </div>
  );
}

function MemberRow(props: { workgroup: string; member: Member }) {
  return (
    <Switch.Root
      class="wg-row wg-switch"
      checked={props.member.member}
      onCheckedChange={(d) =>
        void act(() =>
          api.workgroups.setMember({
            workgroup: props.workgroup,
            kind: props.member.kind,
            member: props.member.id,
            on: d.checked,
          }),
        )
      }
    >
      <Switch.Label class="grow">{props.member.label}</Switch.Label>
      <Switch.Control class="switch">
        <Switch.Thumb class="switch-thumb" />
      </Switch.Control>
      <Switch.HiddenInput />
    </Switch.Root>
  );
}

/** What holds for every workgroup at once. */
function WholeDetail() {
  const persona = createAsync(() => personaQuery());
  return (
    <div class="wg-detail">
      <header class="wg-title" style={{ "--c": "var(--ink)" }}>
        <span class="wg-swatch" />
        <h2>全体</h2>
      </header>
      <p class="sub">ここで決めることは、どのワークグループを選んでいても同じように効く。</p>
      <Show when={persona()}>{(p) => <ZoneSection timezone={p().timezone} />}</Show>
    </div>
  );
}

export default function Workgroups() {
  const [params, setParams] = useSearchParams<{ id?: string }>();
  const all = createAsync(() => workgroupsQuery());
  const editable = () => (all() ?? []).filter((c) => c.id !== WHOLE);
  const currentId = () => editable().find((c) => c.id === params.id)?.id ?? WHOLE;
  const detail = createAsync(async () => {
    const id = currentId();
    return id !== WHOLE ? workgroupQuery(id) : undefined;
  });
  setView("設定 · ワークグループ");

  const [adding, setAdding] = createSignal(false);
  const [editing, setEditing] = createSignal(false);
  const [deleting, setDeleting] = createSignal(false);

  return (
    <>
      <div class="page-head">
        <div>
          <h1>ワークグループ</h1>
          <p class="sub">
            仕事・個人のように、頼みごとと使うものを分ける。上の帯でワークグループを選ぶと、会話と右の画面はそこに入るものだけになり、秘書もその中で動く。「すべて」はいつもあり、どのワークグループのものも見える。どのワークグループにも同じように効く設定は「全体」で決める。
          </p>
        </div>
        <Modal
          open={adding()}
          onOpenChange={setAdding}
          title="ワークグループを足す"
          description="名前と、選んでいるときの帯の色を決める。中身はあとから決められる。"
          trigger={
            <Dialog.Trigger class="btn primary">
              <Icon svg={icons.plus} size={14} />
              ワークグループを足す
            </Dialog.Trigger>
          }
        >
          <WorkgroupForm
            name=""
            color={COLORS[0][1]}
            submit="足す"
            onSubmit={async (name, color) => {
              const c = await act(() => api.workgroups.addWorkgroup({ name, color }), "ワークグループを足した");
              if (c) {
                setAdding(false);
                setParams({ id: c.id });
              }
            }}
          />
        </Modal>
      </div>

      <div class="wg-layout">
        <nav class="wg-list" aria-label="ワークグループの一覧">
          <button
            type="button"
            class="wg-pick"
            classList={{ on: currentId() === WHOLE }}
            aria-pressed={currentId() === WHOLE}
            onClick={() => setParams({ id: WHOLE })}
          >
            <Icon svg={icons.world} size={14} />
            全体
          </button>
          <For each={editable()}>
            {(c) => (
              <button
                type="button"
                class="wg-pick"
                classList={{ on: c.id === currentId() }}
                aria-pressed={c.id === currentId()}
                onClick={() => setParams({ id: c.id })}
              >
                <span class="dot" style={{ background: c.color }} />
                {c.name}
              </button>
            )}
          </For>
        </nav>

        <Show when={currentId() === WHOLE}>
          <WholeDetail />
        </Show>
        <Show when={currentId() !== WHOLE && detail()}>
          {(d) => {
            const workgroup = () => d().workgroup;
            const id = () => workgroup()?.id ?? "";
            return (
              <div class="wg-detail">
                <header class="wg-title" style={{ "--c": workgroup()?.color ?? "var(--ink)" }}>
                  <span class="wg-swatch" />
                  <h2>{workgroup()?.name}</h2>
                  <div class="row wg-title-actions">
                    <Modal
                      open={editing()}
                      onOpenChange={setEditing}
                      title="名前と色を変える"
                      trigger={
                        <Dialog.Trigger class="btn small">
                          <Icon svg={icons.pencil} size={14} />
                          名前と色
                        </Dialog.Trigger>
                      }
                    >
                      <WorkgroupForm
                        name={workgroup()?.name ?? ""}
                        color={workgroup()?.color ?? COLORS[0][1]}
                        submit="変える"
                        onSubmit={async (name, color) => {
                          const ok = await act(() => api.workgroups.updateWorkgroup({ id: id(), name, color }));
                          if (ok) setEditing(false);
                        }}
                      />
                    </Modal>
                    <Modal
                      open={deleting()}
                      onOpenChange={setDeleting}
                      title={`「${workgroup()?.name}」を消す`}
                      description="ワークグループを消しても、記録と記憶は消えない。その印が付いていたものは「すべて」で見える。"
                      trigger={
                        <Dialog.Trigger class="btn small danger">
                          <Icon svg={icons.trash} size={14} />
                          消す
                        </Dialog.Trigger>
                      }
                    >
                      <div class="dialog-actions">
                        <Dialog.CloseTrigger class="btn" type="button">
                          やめる
                        </Dialog.CloseTrigger>
                        <button
                          type="button"
                          class="btn danger"
                          onClick={async () => {
                            const ok = await act(() => api.workgroups.deleteWorkgroup({ id: id() }), "ワークグループを消した");
                            if (ok) {
                              setDeleting(false);
                              setParams({ id: undefined });
                            }
                          }}
                        >
                          消す
                        </button>
                      </div>
                    </Modal>
                  </div>
                </header>

                <section class="pref">
                  <div class="pref-head">
                    <h3>予定の見え方</h3>
                    <p class="sub">
                      このワークグループを選んでいるとき、それぞれのカレンダーの予定をどう出すか。出さない予定も、秘書は空いていない時間として扱う。
                    </p>
                  </div>
                  <Show when={d().calendars.length > 0} fallback={<p class="sub">カレンダーがまだない。サービスで Google をつなぐと出てくる。</p>}>
                    <For each={d().calendars}>
                      {(cal) => (
                        <div class="wg-row">
                          <span class="grow">{cal.name}</span>
                          <SegmentGroup.Root
                            class="segs"
                            aria-label={`${cal.name}の出し方`}
                            value={MODES.find(([m]) => m === cal.mode)?.[1] ?? "show"}
                            onValueChange={(v) => {
                              const mode = MODES.find(([, key]) => key === v.value)?.[0];
                              if (mode !== undefined)
                                void act(() =>
                                  api.workgroups.setCalendarMode({ workgroup: id(), calendar: cal.calendar, mode }),
                                );
                            }}
                          >
                            <SegmentGroup.Indicator class="segs-indicator" />
                            <For each={MODES}>
                              {([, key, label]) => (
                                <SegmentGroup.Item value={key} class="segs-item">
                                  <SegmentGroup.ItemText>{label}</SegmentGroup.ItemText>
                                  <SegmentGroup.ItemControl />
                                  <SegmentGroup.ItemHiddenInput />
                                </SegmentGroup.Item>
                              )}
                            </For>
                          </SegmentGroup.Root>
                        </div>
                      )}
                    </For>
                  </Show>
                </section>

                <section class="pref">
                  <div class="pref-head">
                    <h3>使うアカウント</h3>
                    <p class="sub">
                      このワークグループで秘書がメールを送ったり予定を入れたりするとき、どのアカウントを使うか。決めておくと、秘書はいつもそのアカウントを使う。
                    </p>
                  </div>
                  <For each={d().accounts}>{(a) => <AccountRow workgroup={id()} account={a} />}</For>
                </section>

                <section class="pref">
                  <div class="pref-head">
                    <h3>このワークグループに入るもの</h3>
                    <p class="sub">どこから来たものをこのワークグループとして受け、何を確かめずに使うか。一つのものをいくつのワークグループに入れてもよい。</p>
                  </div>
                  <For each={KINDS}>
                    {([kind, title, meaning]) => {
                      const members = () => d().members.filter((m) => m.kind === kind);
                      return (
                        <Show when={members().length > 0}>
                          <div class="wg-group">
                            <h4>{title}</h4>
                            <p class="sub">{meaning}</p>
                            <For each={members()}>{(m) => <MemberRow workgroup={id()} member={m} />}</For>
                          </div>
                        </Show>
                      );
                    }}
                  </For>
                </section>
              </div>
            );
          }}
        </Show>
      </div>
    </>
  );
}
