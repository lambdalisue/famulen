import { A, createAsync, query } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import AskAbout from "~/components/AskAbout";
import LogSheet from "~/components/logs/LogSheet";
import LookoutLog from "~/components/logs/LookoutLog";
import LookoutEdit from "~/components/LookoutEdit";
import { ROUTES } from "~/components/LookoutFields";
import LookoutForm from "~/components/LookoutForm";
import LookoutInterval from "~/components/LookoutInterval";
import PastPurchases from "~/components/PastPurchases";
import RoleModel from "~/components/RoleModel";
import { type LookoutDefault, LookoutRoute, type LookoutSource } from "~/gen/famulen/v1/lookout_pb";
import { Role } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { setView } from "~/lib/view";
import {
  Dialog,
  DialogTrigger,
  Disclosure,
  EmptyState,
  Icon,
  IconButton,
  Keyed,
  List,
  NumberField,
  PageHeader,
  Section,
  SubSection,
  Switch,
  whenConfirmed,
} from "~/components/ui";

const lookoutQuery = query(() => api.lookout.getLookout({}), "lookout");

/** How often each feed is read by sources without their own interval, and the sources that have one. */
function IntervalDefaults(props: { defaults: LookoutDefault[]; sources: LookoutSource[] }) {
  const own = () => props.sources.filter((s) => s.ownInterval);
  return (
    <Section
      title="既定の間隔"
      note="間隔を上書きしていない見張りは、読むものの種類ごとにこの間隔で読む。上書きは、サービスの頁や上の一覧でする。"
    >
      <div class="interval-defaults">
        <Keyed each={props.defaults} by={(d) => d.feed}>
          {(d) => <DefaultInterval value={d()} />}
        </Keyed>
      </div>
      <SubSection title="その見張りだけの間隔で読んでいるもの">
        <Show when={own().length > 0} fallback={<EmptyState>どの見張りも既定に従っている</EmptyState>}>
          <ul class="interval-own" aria-label="間隔を決めている見張り">
            <Keyed each={own()} by={(s) => s.id}>
              {(s) => (
                <li>
                  <span class="grow">{s().name}</span>
                  <LookoutInterval source={s()} defaults={props.defaults} place="own" label={`${s().name}だけの間隔（分）`} />
                </li>
              )}
            </Keyed>
          </ul>
        </Show>
      </SubSection>
    </Section>
  );
}

/** One feed's default interval; saved on commit. */
function DefaultInterval(props: { value: LookoutDefault }) {
  const d = () => props.value;
  return (
    <NumberField
      label={`${d().label}の既定の間隔（分）`}
      min={1}
      value={d().intervalMinutes}
      unit="分ごと"
      onSave={(intervalMinutes) => void act(() => api.lookout.setLookoutDefault({ feed: d().feed, intervalMinutes }), "既定の間隔を変えた")}
    />
  );
}

/** Watching the outside world and passing on only what concerns the owner. */
export default function Lookout() {
  setView("設定", "見張り");
  const w = createAsync(() => lookoutQuery());
  const [adding, setAdding] = createSignal(false);

  return (
    <>
      <PageHeader
        title="見張り"
        note={
          <>
            <p>
              メールやタスクを見張るエージェント。届いたものをまとめて一度に読み、あなたに関わるか、どれほど急ぐかを付け、同じ話をまとめて、秘書にすぐ渡すか朝のまとめに回すかを決める。
            </p>
            <p>
              あなたに関わるかどうかは、記録（まだ済んでいないタスク・相手・物と契約）と記憶から作った手がかりで決める。関係ないものが届いたら、秘書に「これは関係ない」と伝えるか、ログで「要らなかった」を返す。どちらも記憶に入り、夢のあとの判断から効く。どう判断したかは、頁の下の「ログを見る」で開く。
            </p>
            <p>
              この 24 時間：<b>{String(w()?.seen ?? 0n)}</b> 件を見て、<b>{String(w()?.passed ?? 0n)}</b>{" "}
              件を秘書に渡した。今日モデルを使ったのは {String(w()?.modelCallsToday ?? 0n)} 回。
            </p>
          </>
        }
      />
      <Disclosure title="いま判断に使っている手がかりを見る">
        <pre class="mono sub lookout-profile">{(w()?.profile ?? "").trim() || "まだない"}</pre>
      </Disclosure>
      <div class="prefs">
        <RoleModel
          role={Role.LOOKOUT}
          cap="見張るものを読むたびに、届いたぶんをまとめて 1 回使う。上限に達した日は、それ以上読み分けず、残りを朝のまとめに回す。値段のあるモデルは円で、ないモデルは回数で数える。"
        />
        <Section
          title="見張るもの"
          note="見張りが読むところと、どれほどの間隔で読むか。間隔は下の「既定の間隔」に従うか、見張りごとに上書きする。"
          action={
            <Dialog
              open={adding()}
              onOpenChange={setAdding}
              title="見張りを足す"
              description="つなぎ先を選ぶと、そこに届いたものを見張る。"
              trigger={
                <DialogTrigger variant="primary" icon={icons.plus}>
                  見張りを足す
                </DialogTrigger>
              }
            >
              <LookoutForm
                defaults={w()?.defaults ?? []}
                presets={w()?.presets ?? []}
                sources={w()?.sources ?? []}
                onDone={() => setAdding(false)}
              />
            </Dialog>
          }
        >
          <List>
            <Keyed each={w()?.sources ?? []} by={(s) => s.id}>
              {(s) => (
                <div class="item wraps-phone" role="listitem">
                  <Icon svg={icons.eye} size={16} />
                  <div class="grow lookout-source">
                    <div class="item-title">{s().name}</div>
                    <Show when={ROUTES.find(([r]) => r === s().route)}>
                      {(route) => <span class="sub">{route()[0] === LookoutRoute.DIGEST ? "朝のまとめへ" : route()[1]}</span>}
                    </Show>
                    <Show when={s().places.length > 0}>
                      <span class="sub">
                        {s()
                          .places.map((p) => p.name)
                          .join("・")}{" "}
                        だけ
                      </span>
                    </Show>
                    <LookoutInterval source={s()} defaults={w()?.defaults ?? []} place="sources" />
                  </div>
                  <Switch
                    checked={s().enabled}
                    onChange={(enabled) =>
                      void act(() => api.lookout.setLookoutEnabled({ id: s().id, enabled }), enabled ? "見張りを始めた" : "止めた")
                    }
                  >
                    {s().enabled ? "見張っている" : "止めている"}
                    <span class="sr">（{s().name}）</span>
                  </Switch>
                  <AskAbout framed what={`見張り ${s().name}`} />
                  <LookoutEdit source={s()} defaults={w()?.defaults ?? []} sources={w()?.sources ?? []} />
                  <IconButton
                    icon={icons.trash}
                    label={`${s().name}を外す`}
                    variant="danger"
                    onClick={() =>
                      void whenConfirmed(`見張り「${s().name}」を外す`, "外す", () =>
                        act(() => api.lookout.removeLookout({ id: s().id }), "外した"),
                      )
                    }
                  />
                </div>
              )}
            </Keyed>
            <Show when={(w()?.sources ?? []).length === 0}>
              <EmptyState>まだない</EmptyState>
            </Show>
          </List>
          <Show when={(w()?.presets ?? []).length > 0}>
            <SubSection title="押すだけで足せるもの">
              <div class="lookout-presets">
                <For each={w()?.presets ?? []}>
                  {(p) => (
                    <button
                      type="button"
                      class="choice"
                      onClick={() =>
                        act(
                          () =>
                            api.lookout.addLookout({
                              name: p.name,
                              connection: p.connection,
                              tool: p.tool,
                              args: p.args,
                              intervalMinutes: 0,
                            }),
                          "見張りに足した",
                        )
                      }
                    >
                      <Icon svg={icons.plus} size={16} />
                      <span class="grow">
                        <b>{p.name}</b>
                        <br />
                        <span class="sub">{p.what}</span>
                      </span>
                    </button>
                  )}
                </For>
              </div>
            </SubSection>
          </Show>
        </Section>
        <IntervalDefaults defaults={w()?.defaults ?? []} sources={w()?.sources ?? []} />
        <PastPurchases />
      </div>
      <LogSheet title="見張りのログ">
        <LookoutLog />
      </LogSheet>
    </>
  );
}
