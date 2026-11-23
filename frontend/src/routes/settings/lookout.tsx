import { Dialog } from "@ark-ui/solid/dialog";
import { NumberInput } from "@ark-ui/solid/number-input";
import { A, createAsync, query } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import Keyed from "~/components/Keyed";
import Icon from "~/components/Icon";
import Modal from "~/components/Modal";
import LogSheet from "~/components/logs/LogSheet";
import LookoutLog from "~/components/logs/LookoutLog";
import LookoutEdit from "~/components/LookoutEdit";
import { ROUTES } from "~/components/LookoutFields";
import LookoutForm from "~/components/LookoutForm";
import LookoutInterval from "~/components/LookoutInterval";
import RoleModel from "~/components/RoleModel";
import { type LookoutDefault, LookoutRoute, type LookoutSource } from "~/gen/famulen/v1/lookout_pb";
import { Role } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { createDraft } from "~/lib/draft";
import * as icons from "~/lib/icons";
import { setSelection, setView } from "~/lib/view";

const lookoutQuery = query(() => api.lookout.getLookout({}), "lookout");

/** How often each feed is read by sources without their own interval, and the sources that have one. */
function IntervalDefaults(props: { defaults: LookoutDefault[]; sources: LookoutSource[] }) {
  const own = () => props.sources.filter((s) => s.ownInterval);
  return (
    <section class="pref">
      <div class="pref-head">
        <h2>既定の間隔</h2>
        <p class="sub">
          間隔を上書きしていない見張りは、読むものの種類ごとにこの間隔で読む。上書きは、サービスの頁や上の一覧でする。
        </p>
      </div>
      <div class="interval-defaults">
        <Keyed each={props.defaults} by={(d) => d.feed}>
          {(d) => <DefaultInterval value={d()} />}
        </Keyed>
      </div>
      <div class="pref-sub">
        <h3>その見張りだけの間隔で読んでいるもの</h3>
        <Show when={own().length > 0} fallback={<p class="sub">どの見張りも既定に従っている。</p>}>
          <ul class="interval-own" aria-label="間隔を決めている見張り">
            <Keyed each={own()} by={(s) => s.id}>
              {(s) => (
                <li>
                  <span class="grow">
                    {s().name}
                  </span>
                  <LookoutInterval source={s()} defaults={props.defaults} place="own" label={`${s().name}だけの間隔（分）`} />
                </li>
              )}
            </Keyed>
          </ul>
        </Show>
      </div>
    </section>
  );
}

/** One feed's default interval; saved on commit. */
function DefaultInterval(props: { value: LookoutDefault }) {
  const d = () => props.value;
  const [shown, setShown] = createDraft(() => String(d().intervalMinutes));
  return (
    <NumberInput.Root
      class="interval-default"
      min={1}
      value={shown()}
      onValueChange={(v) => setShown(v.value)}
      onValueCommit={(v) => {
        const next = Math.trunc(v.valueAsNumber);
        if (!Number.isFinite(next) || next < 1) {
          setShown(String(d().intervalMinutes));
          return;
        }
        if (next !== d().intervalMinutes)
          void act(() => api.lookout.setLookoutDefault({ feed: d().feed, intervalMinutes: next }), "既定の間隔を変えた");
      }}
    >
      <NumberInput.Label>{`${d().label}の既定の間隔（分）`}</NumberInput.Label>
      <div class="row">
        <NumberInput.Control>
          <NumberInput.Input class="input" inputmode="numeric" />
        </NumberInput.Control>
        <span class="sub">分ごと</span>
      </div>
    </NumberInput.Root>
  );
}

/** Watching the outside world and passing on only what concerns the owner. */
export default function Lookout() {
  setView("設定 · 見張り");
  const w = createAsync(() => lookoutQuery());
  const [adding, setAdding] = createSignal(false);

  return (
    <>
      <h1>見張り</h1>
      <p class="sub">
        メールやタスクを見張るエージェント。届いたものをまとめて一度に読み、あなたに関わるか、どれほど急ぐかを付け、同じ話をまとめて、秘書にすぐ渡すか朝のまとめに回すかを決める。
      </p>
      <p class="sub">
        あなたに関わるかどうかは、記録（まだ済んでいないタスク・相手・物と契約）と記憶から作った手がかりで決める。関係ないものが届いたら、秘書に「これは関係ない」と伝えるか、ログで「要らなかった」を返す。どちらも記憶に入り、夢のあとの判断から効く。どう判断したかは、頁の下の「ログを見る」で開く。
      </p>
      <p class="sub">
        この 24 時間：<b>{String(w()?.seen ?? 0n)}</b> 件を見て、<b>{String(w()?.passed ?? 0n)}</b>{" "}
        件を秘書に渡した。今日モデルを使ったのは {String(w()?.modelCallsToday ?? 0n)} 回。
      </p>
      <details class="intro-more">
        <summary class="sub">いま判断に使っている手がかりを見る</summary>
        <pre class="mono sub lookout-profile">{(w()?.profile ?? "").trim() || "まだない"}</pre>
      </details>
      <div class="prefs">
        <RoleModel
          role={Role.LOOKOUT}
          cap="見張るものを読むたびに、届いたぶんをまとめて 1 回使う。上限に達した日は、それ以上読み分けず、残りを朝のまとめに回す。値段のあるモデルは円で、ないモデルは回数で数える。"
        />
        <section class="pref">
          <div class="page-head">
            <div class="pref-head">
              <h2>見張るもの</h2>
              <p class="sub">見張りが読むところと、どれほどの間隔で読むか。間隔は下の「既定の間隔」に従うか、見張りごとに上書きする。</p>
            </div>
            <Modal
              open={adding()}
              onOpenChange={setAdding}
              title="見張りを足す"
              description="つなぎ先を選ぶと、そこに届いたものを見張る。"
              trigger={
                <Dialog.Trigger class="btn primary">
                  <Icon svg={icons.plus} size={14} />
                  見張りを足す
                </Dialog.Trigger>
              }
            >
              <LookoutForm defaults={w()?.defaults ?? []} presets={w()?.presets ?? []} sources={w()?.sources ?? []} onDone={() => setAdding(false)} />
            </Modal>
          </div>
          <div class="list">
            <Keyed each={w()?.sources ?? []} by={(s) => s.id}>
              {(s) => (
                <div class="item" onClick={() => setSelection(`見張り ${s().name}`)}>
                  <Icon svg={icons.eye} size={16} />
                  <div class="grow lookout-source">
                    <b>{s().name}</b>
                    <Show when={ROUTES.find(([r]) => r === s().route)}>
                      {(route) => <span class="sub">{route()[0] === LookoutRoute.DIGEST ? "朝のまとめへ" : route()[1]}</span>}
                    </Show>
                    <Show when={s().places.length > 0}>
                      <span class="sub">{s().places.map((p) => p.name).join("・")} だけ</span>
                    </Show>
                    <LookoutInterval source={s()} defaults={w()?.defaults ?? []} place="sources" />
                  </div>
                  <button
                    type="button"
                    class={`btn small${s().enabled ? " primary" : ""}`}
                    aria-pressed={s().enabled}
                    onClick={() =>
                      act(
                        () => api.lookout.setLookoutEnabled({ id: s().id, enabled: !s().enabled }),
                        s().enabled ? "止めた" : "見張りを始めた",
                      )
                    }
                  >
                    {s().enabled ? "見張っている" : "止めている"}
                  </button>
                  <LookoutEdit source={s()} defaults={w()?.defaults ?? []} sources={w()?.sources ?? []} />
                  <button
                    type="button"
                    class="btn small danger"
                    aria-label="外す"
                    onClick={() => act(() => api.lookout.removeLookout({ id: s().id }), "外した")}
                  >
                    <Icon svg={icons.trash} size={12} />
                  </button>
                </div>
              )}
            </Keyed>
            <Show when={(w()?.sources ?? []).length === 0}>
              <p class="empty">まだない</p>
            </Show>
          </div>
          <Show when={(w()?.presets ?? []).length > 0}>
            <div class="pref-sub">
              <h3>押すだけで足せるもの</h3>
              <div class="lookout-presets">
                <For each={w()?.presets ?? []}>
                  {(p) => (
                    <button
                      type="button"
                      class="card lookout-preset"
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
            </div>
          </Show>
        </section>
        <IntervalDefaults defaults={w()?.defaults ?? []} sources={w()?.sources ?? []} />
      </div>
      <LogSheet title="見張りのログ">
        <LookoutLog />
      </LogSheet>
    </>
  );
}
