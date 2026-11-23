import { RadioGroup } from "@ark-ui/solid/radio-group";
import { createAsync, query, useSearchParams } from "@solidjs/router";
import { For, Show, createSignal } from "solid-js";
import ZoneSection from "~/components/ZoneSection";
import { CalendarMode } from "~/gen/famulen/v1/calendar_pb";
import { type AccountSetting, type Member, MemberKind } from "~/gen/famulen/v1/workgroups_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { workgroupsQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { setView } from "~/lib/view";
import { Button, Dialog, DialogActions, DialogTrigger, Field, Icon, List, ListRow, PageHeader, Section, SegmentedOf, Select, Switch, TextField, whenConfirmed } from "~/components/ui";

const COLORS: [string, string][] = [
  ["青", "#2a78d6"],
  ["橙", "#E0622F"],
  ["青緑", "#0F6E68"],
  ["紫", "#6250d6"],
  ["緑", "#008300"],
];

const MODES: [CalendarMode, string][] = [
  [CalendarMode.SHOW, "出す"],
  [CalendarMode.FAINT, "薄く出す"],
  [CalendarMode.HIDE, "出さない"],
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
      <Field label="名前">
        <TextField name="name" placeholder="趣味" required value={name()} onInput={(e) => setName(e.currentTarget.value)} />
      </Field>
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
      <DialogActions submit={props.submit} />
    </form>
  );
}

function AccountRow(props: { workgroup: string; account: AccountSetting }) {
  return (
    <ListRow
      title={props.account.label}
      end={
        <Select
          aria-label={props.account.label}
          onChange={(e) => {
            const connection = e.currentTarget.value;
            void act(() => api.workgroups.setAccount({ workgroup: props.workgroup, purpose: props.account.purpose, connection }));
          }}
        >
          <option value="" selected={!props.account.connection}>
            決めない
          </option>
          <For each={props.account.choices}>
            {(c) => (
              <option value={c.id} selected={c.id === props.account.connection}>
                {c.label}
              </option>
            )}
          </For>
        </Select>
      }
    />
  );
}

function MemberRow(props: { workgroup: string; member: Member }) {
  return (
    <Switch
      end
      checked={props.member.member}
      onChange={(on) => void act(() => api.workgroups.setMember({ workgroup: props.workgroup, kind: props.member.kind, member: props.member.id, on }))}
    >
      {props.member.label}
    </Switch>
  );
}

/** What holds for every workgroup at once. */
function WholeDetail() {
  const owner = useOwner();
  return (
    <div class="wg-detail">
      <header class="wg-title" style={{ "--c": "var(--ink)" }}>
        <span class="wg-swatch" />
        <h2>全体</h2>
      </header>
      <p class="sub">ここで決めることは、どのワークグループを選んでいても同じように効く。</p>
      <Show when={owner.persona()}>{(p) => <ZoneSection timezone={p().timezone} />}</Show>
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
  setView("設定", "ワークグループ");

  const [adding, setAdding] = createSignal(false);
  const [editing, setEditing] = createSignal(false);

  return (
    <>
      <div class="page-head">
        <div>
          <PageHeader title="ワークグループ" note="仕事・個人のように、頼みごとと使うものを分ける。上の帯でワークグループを選ぶと、会話と右の画面はそこに入るものだけになり、秘書もその中で動く。「すべて」はいつもあり、どのワークグループのものも見える。どのワークグループにも同じように効く設定は「全体」で決める。" />
        </div>
        <Dialog
          open={adding()}
          onOpenChange={setAdding}
          title="ワークグループを足す"
          description="名前と、選んでいるときの帯の色を決める。中身はあとから決められる。"
          trigger={
            <DialogTrigger variant="primary" icon={icons.plus}>
ワークグループを足す
            </DialogTrigger>
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
        </Dialog>
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
                    <Dialog
                      open={editing()}
                      onOpenChange={setEditing}
                      title="名前と色を変える"
                      trigger={
                        <DialogTrigger small icon={icons.pencil}>
名前と色
                        </DialogTrigger>
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
                    </Dialog>
                    <Button
                      small
                      variant="danger"
                      icon={icons.trash}
                      onClick={() =>
                        void whenConfirmed(
                          `「${workgroup()?.name}」を消す`,
                          "消す",
                          async () => {
                            if (await act(() => api.workgroups.deleteWorkgroup({ id: id() }), "ワークグループを消した")) setParams({ id: undefined });
                          },
                          "ワークグループを消しても、記録と記憶は消えない。その印が付いていたものは「すべて」で見える。",
                        )
                      }
                    >
                      消す
                    </Button>
                  </div>
                </header>

                <Section title="予定の見え方" level={3} note="このワークグループを選んでいるとき、それぞれのカレンダーの予定をどう出すか。出さない予定も、秘書は空いていない時間として扱う。">
                  <Show when={d().calendars.length > 0} fallback={<p class="sub">カレンダーがまだない。サービスで Google をつなぐと出てくる。</p>}>
                    <List>
                    <For each={d().calendars}>
                      {(cal) => (
                        <ListRow
                          title={cal.name}
                          end={
                            <SegmentedOf
                              label={`${cal.name}の出し方`}
                              items={MODES}
                              value={cal.mode}
                              onChoose={(mode) => void act(() => api.workgroups.setCalendarMode({ workgroup: id(), calendar: cal.calendar, mode }))}
                            />
                          }
                        />
                      )}
                    </For>
                    </List>
                  </Show>
                </Section>

                <Section title="使うアカウント" level={3} note="このワークグループで秘書がメールを送ったり予定を入れたりするとき、どのアカウントを使うか。決めておくと、秘書はいつもそのアカウントを使う。">
                  <For each={d().accounts}>{(a) => <AccountRow workgroup={id()} account={a} />}</For>
                </Section>

                <Section title="このワークグループに入るもの" level={3} note="どこから来たものをこのワークグループとして受け、何を確かめずに使うか。一つのものをいくつのワークグループに入れてもよい。">
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
                </Section>
              </div>
            );
          }}
        </Show>
      </div>
    </>
  );
}
