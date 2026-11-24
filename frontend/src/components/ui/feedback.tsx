import { type ComponentProps, type JSX, Show, splitProps } from "solid-js";
import { message } from "~/lib/act";
import * as icons from "~/lib/icons";
import { Button } from "./Button";
import Icon from "./Icon";

/**
 * What a tag says about what it stands on: `ok` stands well, `warn` wants a
 * look, `danger` went wrong, `aide` is the secretary's, `said` and `guess`
 * are how a memory came, `label` is the owner's own.
 */
export type Tone = "ok" | "warn" | "danger" | "aide" | "said" | "guess" | "label";

export function Tag(props: ComponentProps<"span"> & { tone?: Tone }) {
  const [own, rest] = splitProps(props, ["tone", "class", "children"]);
  return (
    <span {...rest} class={`tag ${own.tone ?? ""} ${own.class ?? ""}`}>
      {own.children}
    </span>
  );
}

/** What the owner should know about what is below it. */
export function Callout(props: { tone?: "warn" | "danger"; icon?: string; action?: JSX.Element; live?: boolean; children: JSX.Element }) {
  const icon = () => props.icon ?? (props.tone ? icons.alertTriangle : undefined);
  return (
    <div class={`callout ${props.tone ?? ""}`} role={props.live ? (props.tone ? "alert" : "status") : undefined}>
      <Show when={icon()}>{(svg) => <Icon svg={svg()} size={16} />}</Show>
      <div class="grow">{props.children}</div>
      {props.action}
    </div>
  );
}

/** That there is nothing yet, and how something comes to be there. */
export function EmptyState(props: { children: JSX.Element; action?: JSX.Element; class?: string }) {
  return (
    <div class={`empty ${props.class ?? ""}`}>
      <span>{props.children}</span>
      {props.action}
    </div>
  );
}

export function Loading() {
  return (
    <p class="empty" role="status">
      読み込んでいる
    </p>
  );
}

/** A read that failed: what went wrong, and a way to try it again. */
export function ErrorState(props: { error: unknown; onRetry?: () => void }) {
  return (
    <Callout
      tone="danger"
      live
      action={
        <Show when={props.onRetry}>
          {(retry) => (
            <Button small icon={icons.refresh} onClick={() => retry()()}>
              読み直す
            </Button>
          )}
        </Show>
      }
    >
      <b>読めなかった</b>
      <div class="sub">{message(props.error)}</div>
    </Callout>
  );
}

/** How much of a limit is used. */
export function Meter(props: { label: string; value: number; max: number; unit: string }) {
  return (
    <div class="meter-row">
      <span>{props.label}</span>
      <span class="mono">
        {props.value} / {props.max} {props.unit}
      </span>
      <div class="meter" role="meter" aria-label={props.label} aria-valuenow={props.value} aria-valuemin={0} aria-valuemax={props.max}>
        <div style={{ width: `${Math.min(100, Math.floor((props.value * 100) / props.max))}%` }} />
      </div>
    </div>
  );
}
