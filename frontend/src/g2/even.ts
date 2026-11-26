/**
 * The Even App's bridge to the glasses, as the G2 page uses it: pages to
 * draw, the microphone, and what the owner does on the temple or the ring.
 */

import {
  AudioInputSource,
  CreateStartUpPageContainer,
  type EvenAppBridge,
  type EvenHubEvent,
  ImageContainerProperty,
  ImageRawDataUpdate,
  ImageRawDataUpdateResult,
  OsEventTypeList,
  RebuildPageContainer,
  StartUpPageCreateResult,
  TextContainerProperty,
  TextContainerUpgrade,
  waitForEvenAppBridge,
} from "@evenrealities/even_hub_sdk";
import type { Bridge, Layout } from "./page";

/** What the owner did, or what the glasses tell. */
export type Input =
  | { kind: "up" | "down" | "tap" | "double" | "long" }
  | { kind: "sound"; pcm: Uint8Array }
  /** The glasses cleared the page, as when the exit dialog opens. */
  | { kind: "cleared" }
  /** The owner chose to stay in the exit dialog. */
  | { kind: "stayed" }
  | { kind: "gone" };

/** The glasses, as a test or the Even App gives them to the page. */
export interface Glasses extends Bridge {
  microphone(on: boolean): Promise<boolean>;
  /** Ask the owner whether to leave, in the glasses' own dialog. */
  leave(): Promise<void>;
  onInput(listener: (input: Input) => void): void;
}

declare global {
  interface Window {
    /** Glasses a test hands in instead of the Even App's. */
    famulenGlasses?: Glasses;
  }
}

/** How long one call to the glasses may take before it is given up. */
const PATIENCE_MS = 8_000;

function inTime<T>(call: Promise<T>, fallback: T): Promise<T> {
  return Promise.race([call, new Promise<T>((done) => setTimeout(() => done(fallback), PATIENCE_MS))]);
}

/** What one event from the glasses means; undefined for what the page does not use. */
export function inputOf(event: EvenHubEvent): Input | undefined {
  if (event.audioEvent) return { kind: "sound", pcm: event.audioEvent.audioPcm };
  const item = event.textEvent ?? event.listEvent ?? event.sysEvent;
  if (!item) return undefined;
  // A tap can come with no type, as the ring sends it; a system event with no
  // type and no temple or ring behind it is not one.
  if (item.eventType === undefined) return event.sysEvent && event.sysEvent.eventSource === undefined ? undefined : { kind: "tap" };
  switch (item.eventType) {
    case OsEventTypeList.CLICK_EVENT:
      return { kind: "tap" };
    case OsEventTypeList.SCROLL_TOP_EVENT:
      return { kind: "up" };
    case OsEventTypeList.SCROLL_BOTTOM_EVENT:
      return { kind: "down" };
    case OsEventTypeList.DOUBLE_CLICK_EVENT:
      return { kind: "double" };
    case OsEventTypeList.LONG_PRESS_EVENT:
      return { kind: "long" };
    case OsEventTypeList.FOREGROUND_ENTER_EVENT:
      return { kind: "cleared" };
    case OsEventTypeList.FOREGROUND_EXIT_EVENT:
      return { kind: "stayed" };
    case OsEventTypeList.SYSTEM_EXIT_EVENT:
      return { kind: "gone" };
    default:
      return undefined;
  }
}

/** A layout as the SDK's models of it. */
function containers(layout: Layout) {
  return {
    containerTotalNum: layout.containerTotalNum,
    textObject: layout.textObject.map((t) => new TextContainerProperty(t)),
    imageObject: layout.imageObject.map((i) => new ImageContainerProperty(i)),
  };
}

function fromBridge(bridge: EvenAppBridge): Glasses {
  const listeners: ((input: Input) => void)[] = [];
  // The glasses tell one change twice in a moment; it is acted on once.
  let last = { kind: "", at: 0 };
  bridge.onEvenHubEvent((event) => {
    const input = inputOf(event);
    if (!input) return;
    const now = Date.now();
    if (input.kind !== "sound" && input.kind === last.kind && now - last.at < 120 && ["cleared", "stayed", "gone"].includes(input.kind)) return;
    if (input.kind !== "sound") last = { kind: input.kind, at: now };
    for (const listen of listeners) listen(input);
  });
  const ok = async (call: Promise<boolean>) => inTime(call, false);
  return {
    create: async (layout: Layout) =>
      (await inTime(bridge.createStartUpPageContainer(new CreateStartUpPageContainer(containers(layout))), StartUpPageCreateResult.invalid)) === StartUpPageCreateResult.success,
    rebuild: (layout: Layout) => ok(bridge.rebuildPageContainer(new RebuildPageContainer(containers(layout)))),
    text: (containerID, containerName, content) => ok(bridge.textContainerUpgrade(new TextContainerUpgrade({ containerID, containerName, contentOffset: 0, contentLength: 0, content }))),
    image: async (containerID, containerName, imageData) =>
      (await inTime(bridge.updateImageRawData(new ImageRawDataUpdate({ containerID, containerName, imageData })), ImageRawDataUpdateResult.sendFailed)) ===
      ImageRawDataUpdateResult.success,
    microphone: (on) => ok(bridge.audioControl(on, AudioInputSource.Glasses)),
    leave: async () => {
      await ok(bridge.shutDownPageContainer(1));
    },
    onInput: (listener) => listeners.push(listener),
  };
}

/** The glasses this page runs with: a test's, or the Even App's once it is ready. */
export async function glasses(): Promise<Glasses> {
  if (window.famulenGlasses) return window.famulenGlasses;
  return fromBridge(await waitForEvenAppBridge());
}
