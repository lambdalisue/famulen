import { Dialog } from "@ark-ui/solid/dialog";
import { Title } from "@solidjs/meta";
import { revalidate, useLocation, useNavigate } from "@solidjs/router";
import * as icons from "~/lib/icons";
import { createEffect, createSignal, ErrorBoundary, For, type JSX, on, onCleanup, onMount, Show, Suspense } from "solid-js";
import { chooseWorkgroup, workgroup } from "~/lib/workgroup";
import { own } from "~/lib/colors";
import { followTheme } from "~/lib/theme";
import { useWorkgroups } from "~/lib/workgroups";
import { approvalId, useWaiting } from "~/lib/inbox";
import { look, say } from "~/lib/act";
import { api } from "~/lib/api";
import { followNotices, lapsed } from "~/lib/notices";
import { useParam, withParams } from "~/lib/params";
import { statusQuery } from "~/lib/data";
import { followEvents } from "~/lib/events";
import { createKept } from "~/lib/kept";
import { ASIDE, BOTTOM, SECTIONS, tabOf, wayBack } from "~/lib/tabs";
import { followKeyboard, isNarrow, layoutHeld } from "~/lib/viewport";
import { pageTitle } from "~/lib/view";
import Chat from "./Chat";
import ThemeChoice from "./ThemeChoice";
import Lightbox, { closeImage } from "./Lightbox";
import AideIcon from "./AideIcon";
import { followPortrait } from "./PageIcons";
import { useOwner } from "~/lib/owner";
import StatusPanel from "./StatusPanel";
import { ConfirmHost, ErrorState, Icon, LinkButton, List, Loading, SideSheet, Toast } from "~/components/ui";

/** How the secretary is, in a word; pressing it opens the whole state. */
function Alive() {
  const status = createKept(() => statusQuery());
  const waiting = useWaiting();
  const busy = () => status()?.state === "running";
  return (
    <Dialog.Root lazyMount unmountOnExit>
      <Dialog.Trigger class="alive" aria-label="秘書の状態">
        <span class="dot" classList={{ busy: busy() }} />
        <span role="status">{busy() ? "考えている" : "待っている"}</span>
        <Show when={waiting.count() > 0}>
          <span class="alive-waiting" title="あなたを待っているもの">
            {waiting.count()}
          </span>
        </Show>
        <Icon svg={icons.chevronDown} size={12} />
      </Dialog.Trigger>
      <SideSheet name="status" title="秘書の状態">
        <StatusPanel />
      </SideSheet>
    </Dialog.Root>
  );
}

/** The console around one page: the bars, the workgroups and the chat. */
export default function Shell(props: { children?: JSX.Element }) {
  const location = useLocation();
  const navigate = useNavigate();
  const confirm = useParam("confirm");
  const workgroups = useWorkgroups();
  const owner = useOwner();
  followPortrait();
  const [chatOpen, setChatOpen] = createSignal(false);
  let workgroupSheet: HTMLDialogElement | undefined;

  const tab = () => tabOf(location.pathname);
  const current = () => workgroups()?.find((c) => c.id === workgroup()) ?? workgroups()?.[0];
  const color = () => own(current()?.color ?? "");
  const aide = owner.aide;
  const more = () => !BOTTOM.includes(tab().key);
  const mobileTitle = () => pageTitle() || tab().label;

  /** Bring a request a notice told of before the owner. */
  const showRequest = async (id: string) => {
    closeImage();
    const inbox = await look(() => api.console.getInbox({}));
    if (!inbox) return;
    await revalidate("inbox");
    if (!inbox.approvals.some((a) => a.status === "pending" && a.id.toString() === id)) {
      return say(`確認 #${id} は、答えを待つものの中にない`);
    }
    workgroupSheet?.close();
    if (isNarrow()) setChatOpen(true);
    requestAnimationFrame(() => {
      const card = document.getElementById(approvalId(id));
      card?.scrollIntoView({ block: "nearest" });
      card?.focus({ preventScroll: true });
      // Something open over the conversation keeps it from being reached.
      if (document.activeElement !== card) say(`確認 #${id} が会話で答えを待っている`);
    });
  };
  // The address is put back, so that a reload does not bring it up again.
  onMount(() =>
    createEffect(() => {
      const id = confirm();
      if (!id) return;
      navigate(withParams(`${location.pathname}${location.search}`, { confirm: "" }), { replace: true });
      void showRequest(id);
    }),
  );

  onMount(() => {
    followTheme();
    followEvents(workgroup);
    followKeyboard();
    onCleanup(followNotices((id) => void showRequest(id)));
    void lapsed().then(
      (over) => over && say("この端末に通知が届かなくなった。設定の「通知」で受け取り直す", true),
      () => {},
    );
    // The narrow layout's rules read these from <body>: while typing, the
    // bottom bar gives way to the keyboard.
    const focusIn = (e: FocusEvent) => {
      if (isNarrow() && e.target instanceof HTMLElement && e.target.id === "say") {
        document.body.classList.add("typing");
        setChatOpen(true);
      }
    };
    const focusOut = (e: FocusEvent) => {
      if (e.target instanceof HTMLElement && e.target.id === "say" && !layoutHeld()) document.body.classList.remove("typing");
    };
    document.addEventListener("focusin", focusIn);
    document.addEventListener("focusout", focusOut);
    onCleanup(() => {
      document.removeEventListener("focusin", focusIn);
      document.removeEventListener("focusout", focusOut);
    });
  });
  createEffect(() => {
    document.body.classList.toggle("chat-open", chatOpen());
  });
  createEffect(
    on(
      () => location.pathname,
      () => {
        // Closing it may open the conversation, as the focus goes back there.
        closeImage();
        setChatOpen(false);
      },
      { defer: true },
    ),
  );

  const choose = async (id: string) => {
    chooseWorkgroup(id);
    workgroupSheet?.close();
    await revalidate();
  };

  return (
    <div class="app" style={{ "--workgroup": color() }}>
      <Title>
        {tab().label} · {aide()}
      </Title>
      <header id="m-head" class="m-head">
        <Show when={wayBack(location.pathname, location.search)}>
          {(back) => (
            <a class="m-back" href={back().href} aria-label={`${back().label}へ戻る`}>
              <Icon svg={icons.chevronLeft} size={26} />
            </a>
          )}
        </Show>
        <h1 class="m-title">{mobileTitle()}</h1>
        <button
          type="button"
          class="m-wg"
          aria-label={`ワークグループを選ぶ（いまは${current()?.name ?? ""}）`}
          onClick={() => workgroupSheet?.showModal()}
        >
          <span class="dot" style={{ background: color() }} />
          {current()?.name}
          <Icon svg={icons.chevronDown} size={16} />
        </button>
      </header>
      <div class="body">
        <Chat open={chatOpen()} setOpen={setChatOpen} />
        <div id="main" class="main">
          <header class="topbar">
            <span class="logo">
              <AideIcon size={32} />
            </span>
            <span class="brand">
              <b>{aide()}</b>
              <small>Famulen</small>
            </span>
            <div class="wg-tabs" role="group" aria-label="ワークグループ">
              <For each={workgroups()}>
                {(c) => (
                  <button
                    type="button"
                    aria-pressed={c.id === current()?.id}
                    class={`wg-btn${c.id === current()?.id ? " on" : ""}`}
                    onClick={() => choose(c.id)}
                  >
                    <span class="dot" style={{ background: own(c.color) }} />
                    {c.name}
                  </button>
                )}
              </For>
              <a class="wg-btn" href="/settings/workgroups" aria-label="ワークグループを足す">
                <Icon svg={icons.plus} size={14} />
              </a>
            </div>
            <span class="wg-hint">
              {current()?.id === "all" || !current()
                ? "すべてのワークグループを使う"
                : `${current()?.name}のものだけを使い、ほかの予定は薄く出す`}
            </span>
            <Alive />
            <ThemeChoice label="テーマの切り替え" brief />
          </header>
          <nav class="tabs" aria-label="セクション">
            <For each={SECTIONS}>
              {(t) => (
                <a class={`tab${t.key === tab().key ? " on" : ""}`} href={t.path} aria-current={t.key === tab().key ? "page" : undefined}>
                  <Icon svg={t.icon} size={16} />
                  {t.label}
                </a>
              )}
            </For>
            <span class="gap" />
            <For each={ASIDE}>
              {(t) => (
                <a class={`tab${t.key === tab().key ? " on" : ""}`} href={t.path} aria-current={t.key === tab().key ? "page" : undefined}>
                  <Icon svg={t.icon} size={16} />
                  {t.label}
                </a>
              )}
            </For>
          </nav>
          <div id="pane" class="pane" data-tab={tab().key}>
            <ErrorBoundary
              fallback={(error, reset) => (
                <div class="pane-error">
                  <ErrorState error={error} onRetry={() => void revalidate().then(reset)} />
                </div>
              )}
            >
              <Suspense fallback={<Loading />}>{props.children}</Suspense>
            </ErrorBoundary>
          </div>
        </div>
      </div>
      <nav id="m-tabbar" class="m-tabbar" aria-label="セクション">
        <For each={SECTIONS.filter((t) => BOTTOM.includes(t.key))}>
          {(t) => (
            <a class={`m-tab${t.key === tab().key ? " on" : ""}`} href={t.path} aria-current={t.key === tab().key ? "page" : undefined}>
              <Icon svg={t.icon} size={24} />
              <span>{t.label}</span>
            </a>
          )}
        </For>
        <a class={`m-tab${more() ? " on" : ""}`} href="/more" aria-current={more() ? "page" : undefined}>
          <Icon svg={icons.layoutGrid} size={24} />
          <span>ほか</span>
        </a>
      </nav>
      <dialog
        ref={workgroupSheet}
        class="sheet"
        aria-label="ワークグループ"
        onClick={(e) => {
          if (e.target === workgroupSheet) workgroupSheet?.close();
        }}
      >
        <div class="sheet-grip" />
        <h2>ワークグループ</h2>
        <p class="sub">会話と画面の両方が絞られる</p>
        <List>
          <For each={workgroups()}>
            {(c) => (
              <button type="button" class="sheet-row" aria-pressed={c.id === current()?.id} onClick={() => choose(c.id)}>
                <span class="dot" style={{ background: own(c.color) }} />
                <b class="grow">{c.name}</b>
                <Show when={c.id === current()?.id}>
                  <Icon svg={icons.check} size={18} />
                </Show>
              </button>
            )}
          </For>
        </List>
        <LinkButton icon={icons.plus} href="/settings/workgroups" onClick={() => workgroupSheet?.close()}>
          ワークグループを足す
        </LinkButton>
      </dialog>
      <Lightbox />
      <ConfirmHost />
      <Toast />
    </div>
  );
}
