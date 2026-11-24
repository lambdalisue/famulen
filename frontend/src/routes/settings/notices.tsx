import { createAsync, revalidate } from "@solidjs/router";
import { createEffect, createSignal, For, type JSX, onMount, Show, startTransition } from "solid-js";
import { NoticeDetail } from "~/gen/famulen/v1/notices_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { dayLabel } from "~/lib/dates";
import { bell, x } from "~/lib/icons";
import { type Lack, lack, leaveNotices, subscription, takeNotices } from "~/lib/notices";
import { noticesQuery } from "~/lib/data";
import { useOwner } from "~/lib/owner";
import { setView } from "~/lib/view";
import { Button, Callout, EmptyState, PageHeader, RadioCards, Section, Tag } from "~/components/ui";

const LACKS: Record<Lack, JSX.Element> = {
  insecure: "この URL では受け取れない。コンソールを https の URL（Tailscale Serve など）で開く。",
  "home-screen": "iPhone と iPad では、共有メニューの「ホーム画面に追加」で足した Famulen を開くと受け取れる（iOS 16.4 から）。",
  unsupported: "このブラウザは通知を受け取れない。",
  denied: "このブラウザで Famulen の通知が拒否されている。ブラウザか端末の設定で許してから、開き直す。",
};

const DETAILS = [
  { id: "number", value: NoticeDetail.NUMBER, label: "番号だけ", note: "例：答えを待っている確認があります" },
  {
    id: "deed",
    value: NoticeDetail.DEED,
    label: "何をするか",
    note: "例：「会社の Gmail」でメールを送る許可を求めています",
  },
  {
    id: "values",
    value: NoticeDetail.VALUES,
    label: "中身まで",
    note: "宛先や文面なども、はじめのほうだけ載せる。秘密が出うると見える確認と、シェルのコマンドでは載せない。",
  },
];

export default function NoticesPage() {
  setView("設定", "通知");
  const owner = useOwner();
  const [endpoint, setEndpoint] = createSignal("");
  // Known only in the browser; until then neither way is offered.
  const [known, setKnown] = createSignal(false);
  const [missing, setMissing] = createSignal<Lack | undefined>();
  const [busy, setBusy] = createSignal(false);
  const notices = createAsync(() => noticesQuery(endpoint()));
  const here = () => notices()?.devices.find((d) => d.here);
  const [chosen, setChosen] = createSignal("");
  createEffect(() => setChosen(DETAILS.find((d) => d.value === notices()?.detail)?.id ?? ""));

  const look = async () => {
    try {
      const found = (await subscription())?.endpoint ?? "";
      setMissing(lack());
      await startTransition(() => setEndpoint(found));
    } catch {
      // A browser that keeps no worker, as in a private window.
      setMissing("unsupported");
    }
    setKnown(true);
  };
  onMount(() => void look());

  const run = async (work: () => Promise<unknown>, done: string) => {
    setBusy(true);
    await act(work, done);
    await look();
    await revalidate("notices");
    setBusy(false);
  };
  const remove = (id: bigint, isHere: boolean) =>
    run(() => (isHere ? leaveNotices(id) : api.notices.removeNoticeDevice({ id })), "外した");

  return (
    <>
      <PageHeader
        title="通知"
        note="秘書や手代があなたの確認を待つとき、コンソールを閉じていても端末に知らせる。通知を開くと、その確認にコンソールで答えられる。"
      />
      <Section title="この端末" label="この端末">
        <Show when={known() && notices()}>
          {(n) => (
            <Show
              when={!missing()}
              fallback={<Callout tone="warn" live>{LACKS[missing() ?? "unsupported"]}</Callout>}
            >
              <Show
                when={n().key}
                fallback={<Callout tone="warn">金庫がないので使えない。FAMULEN_VAULT_KEY を入れて起動する。</Callout>}
              >
                <Show
                  when={here()}
                  fallback={
                    <div class="row">
                      <Button variant="primary" icon={bell} disabled={busy()} onClick={() => void run(() => takeNotices(n().key), "受け取るようにした")}>
                        この端末で受け取る
                      </Button>
                    </div>
                  }
                >
                  {(d) => (
                    <div class="row">
                      <Button icon={bell} disabled={busy()} onClick={() => void run(() => api.notices.tryNoticeDevice({ id: d().id }), "送った")}>
                        試しに送る
                      </Button>
                      <Button disabled={busy()} onClick={() => void remove(d().id, true)}>
                        受け取るのをやめる
                      </Button>
                    </div>
                  )}
                </Show>
              </Show>
            </Show>
          )}
        </Show>
      </Section>
      <Section
        title="通知に載せること"
        note="通知は Apple や Google などの配送サーバーを通る。中身は端末に届くまで暗号になっていて配送サーバーには読めないが、端末のロック画面には出る。"
      >
        <RadioCards
          label="通知に載せること"
          items={DETAILS}
          value={chosen()}
          onChoose={(id) => {
            const detail = DETAILS.find((d) => d.id === id)?.value;
            if (detail === undefined) return;
            const before = chosen();
            setChosen(id);
            void act(() => api.notices.setNoticeDetail({ detail }).then(() => true)).then((saved) => {
              if (!saved) setChosen(before);
            });
          }}
        />
      </Section>
      <Section title="受け取る端末" label="受け取る端末">
        <Show when={(notices()?.devices ?? []).length > 0} fallback={<EmptyState>まだない</EmptyState>}>
          <ul class="places" aria-label="受け取る端末">
            <For each={notices()?.devices}>
              {(d) => (
                <li>
                  <span class="grow">
                    {d.name} <span class="sub">{dayLabel(d.addedAt, owner.tz())}に足した</span>
                  </span>
                  <Show when={d.here}>
                    <Tag>この端末</Tag>
                  </Show>
                  <Show when={d.failed}>
                    <Tag tone="warn">最後の通知を送れなかった</Tag>
                  </Show>
                  <Button
                    variant="danger"
                    small
                    icon={x}
                    aria-label={`${d.name} を外す`}
                    disabled={busy()}
                    onClick={() => void remove(d.id, d.here)}
                  >
                    外す
                  </Button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Section>
    </>
  );
}
