import { batch, createEffect, on, onMount, untrack } from "solid-js";
import { closeImage, imageShown } from "~/components/Lightbox";
import { go, openTalk, type Place, place } from "./talks";
import { narrow } from "./viewport";

/**
 * On a phone the open conversation stands in the browser's history over the
 * page, one step for the chat and one more for what is opened from its list,
 * so that back goes to the list and then folds the chat without leaving the
 * page. A wide screen leaves the history alone.
 */
const KEY = "famulen.chat";

type Step = 1 | 2;
type Mark = { step: Step; at: string; place?: string };

const here = () => window.location.href;

function markHere(): Mark | undefined {
  const mark: unknown = window.history.state?.[KEY];
  if (typeof mark !== "object" || mark === null || !("step" in mark) || !("at" in mark)) return undefined;
  const { step, at } = mark as Mark;
  // A mark copied onto another address by the router is not this page's.
  return at === here() && (step === 1 || step === 2) ? (mark as Mark) : undefined;
}

const stepHere = (): 0 | Step => markHere()?.step ?? 0;

const keyOf = (at: Place) => (at.kind === "talk" ? at.id.toString() : at.kind);

function goBackTo(key: string | undefined) {
  if (key === "new" || key === "all") go({ kind: key });
  else if (key && /^\d+$/.test(key)) openTalk(BigInt(key));
}

/** Show the chat as the entry the browser is on says, all at once, so that nothing in between is followed. */
function show(step: 0 | Step) {
  if (!chat) return;
  const to = chat;
  batch(() => {
    if (step === 1) go({ kind: "list" });
    else if (step === 2) goBackTo(markHere()?.place);
    to.setOpen(step > 0);
  });
}

type Chat = { open: () => boolean; setOpen: (open: boolean) => void; routing: () => boolean };
let chat: Chat | undefined;
/** The address the page last stood on and the chat's step there, to tell a step of the chat's own from the router's. */
let seen: string | undefined;
let stood: 0 | Step = 0;
/** The mark of the entry the chat stands on, and how deep the router counts it, to put back what a replace of the router's drops. */
let standing: { mark: Mark; depth: unknown } | undefined;
/** A step back that the chat took itself, until the browser lands on it. */
let stepping = false;
/** A step forward again, as back only closed the image open over the chat. */
let restoring = false;
/** The entry of the router's gone over to by back or forward, which says whether the chat is open there. */
let traversed: string | undefined;

const stand = () => {
  const mark = markHere();
  standing = mark ? { mark, depth: window.history.state?._depth } : undefined;
};

function wanted(): 0 | Step {
  if (!chat?.open() || !narrow()) return 0;
  return place().kind === "list" ? 1 : 2;
}

/** Where the page behind was scrolled to as the chat opened over it. */
let behind: { el: Element; top: number }[] = [];
const keepBehind = () => {
  const all = [document.scrollingElement, ...document.querySelectorAll("#main, #main *")];
  behind = all.flatMap((el) => (el && el.scrollTop > 0 ? [{ el, top: el.scrollTop }] : []));
};
/** Chromium puts back where it saw the page scrolled as it goes over an entry, and it saw nothing behind the chat. */
const putBehind = () => {
  for (const { el, top } of behind) if (el.isConnected) el.scrollTop = top;
};

/** What makes a page touched, after which the chat is followed. */
const TOUCHING = ["pointerup", "keydown", "mousedown"] as const;
let touching = false;
function followOnTouch() {
  if (touching) return;
  touching = true;
  const now = () => {
    touching = false;
    for (const type of TOUCHING) window.removeEventListener(type, now, true);
    follow();
  };
  for (const type of TOUCHING) window.addEventListener(type, now, true);
}

/** Bring the history to what is shown: one more step for each that opened, and back over each that closed. */
function follow() {
  // The router writes its entry only once it has gone there, after the page shows it.
  if (!chat || stepping || restoring || chat.routing()) return;
  // A pull opens the chat as the finger moves, which does not let a page add to the history: Chromium would pass
  // over such an entry on back. It is followed once the sheet comes to rest, after the finger is lifted.
  if (document.body.classList.contains("chat-moving")) return;
  const want = wanted();
  const has = stepHere();
  if (want < has) {
    stepping = true;
    window.history.go(want - has);
    return;
  }
  const mark = (step: Step): Mark => ({ step, at: here(), place: step === 2 ? keyOf(place()) : undefined });
  if (want === has) {
    if (want === 2 && markHere()?.place !== keyOf(place())) {
      window.history.replaceState({ ...window.history.state, [KEY]: mark(2) }, "");
      stand();
    }
    return;
  }
  // Chromium passes over, on back, an entry a page added before it was ever touched, as when a notice opened the chat.
  if (navigator.userActivation && !navigator.userActivation.hasBeenActive) {
    followOnTouch();
    return;
  }
  if (has === 0) keepBehind();
  // Each keeps how deep the router counts the page under it, as the router is not told of the steps between them.
  for (let step = has + 1; step <= want; step++) window.history.pushState({ ...window.history.state, [KEY]: mark(step as Step) }, "");
  seen = here();
  stood = want;
  stand();
}

/** Follow the chat once a pull of it has come to rest. */
export const settledChat = () => follow();

function popped(e: PopStateEvent) {
  const at = here();
  const step = stepHere();
  // Two entries of one address that the router made, as a link to the page already open does, are its to follow.
  const own = at === seen && (step > 0 || stood > 0);
  const before = standing?.mark.step ?? 0;
  const was = seen;
  seen = at;
  stood = step;
  stand();
  // Another address is the router's: once it has gone there, and was not kept from leaving, the chat follows.
  if (!own) {
    // Between two entries of one address, the router settles on nothing new.
    traversed = at === was ? undefined : at;
    return;
  }
  // A step between entries of the same address is the chat's alone: the router is not told of it.
  e.stopImmediatePropagation();
  // Opened again by going forward, the page behind is where it is now.
  if (before === 0 && step > 0) keepBehind();
  else putBehind();
  if (!chat) return;
  if (restoring) {
    restoring = false;
    return;
  }
  if (stepping) {
    stepping = false;
    follow();
    return;
  }
  // A wide screen has no steps of the chat: they are gone over, the way the browser was going.
  if (!narrow()) {
    if (step > 0) {
      const { [KEY]: _, ...state } = window.history.state ?? {};
      window.history.replaceState(state, "");
      stood = 0;
      stand();
      window.history.go(step > before ? 1 : -1);
    }
    return;
  }
  // Back closes the image open over the chat, and only that.
  if (imageShown() && step < before) {
    closeImage();
    restoring = true;
    window.history.go(1);
    return;
  }
  show(step);
}

// Before the router's, so that the chat's steps can be kept from it.
if (typeof window !== "undefined") {
  window.addEventListener("popstate", popped, { capture: true });
  import.meta.hot?.dispose(() => window.removeEventListener("popstate", popped, { capture: true }));
}

/**
 * Keep the browser's history in step with the chat on a phone. `address` is
 * the router's, and `routing` whether it is on its way somewhere, as it moves
 * the page before it writes the entry.
 */
export function followChatHistory(of: Chat & { address: () => string }) {
  onMount(() => {
    chat = of;
    createEffect(
      on([of.address, of.routing], ([, routing]) => {
        if (routing) return;
        seen = here();
        // A step the chat was taking is moot once the router has gone elsewhere.
        stepping = false;
        restoring = false;
        // The router replaced the entry the chat stands on, at the same depth, and dropped its mark: it is put back for the
        // address the entry has now. Closed, the chat steps back to the address the steps beneath it were taken at.
        if (standing && stepHere() === 0 && window.history.state?._depth === standing.depth && chat?.open()) {
          window.history.replaceState({ ...window.history.state, [KEY]: { ...standing.mark, at: here() } }, "");
        }
        stood = stepHere();
        stand();
        const came = traversed === here();
        traversed = undefined;
        // Come back from another page, or read again, onto where the chat was open: it shows as it was.
        if (stood > 0 && narrow()) {
          keepBehind();
          untrack(() => show(stood));
        } else if (came && narrow()) {
          // An entry of the same page with no step of the chat, as what was opened from it, shows the page alone.
          closeImage();
          untrack(() => show(0));
        }
        untrack(follow);
      }),
    );
    createEffect(on([of.open, place, narrow], () => follow()));
  });
}
