import { dateText } from "~/lib/dates";
import { createAsync } from "@solidjs/router";
import { createEffect, createSignal, For, Match, Show, Switch as Cases } from "solid-js";
import * as icons from "~/lib/icons";
import {
  choiceOf,
  type FieldLike,
  type FieldSpec,
  type FieldValue,
  linkOf,
  linkTo,
  moneyText,
  OWNER,
  SECRETARY,
  WORKER,
} from "~/lib/records";
import RecordChip from "./links/RecordChip";
import { useOwner } from "~/lib/owner";
import RecordPicker from "./RecordPicker";
import { Icon, Select, Switch, Tag, TextField } from "~/components/ui";

const one = (value: string): FieldValue => ({ value, values: [] });
const many = (values: string[]): FieldValue => ({ value: "", values });

/** A field's value as it is read. */
export function FieldView(props: { type: FieldSpec; field: FieldLike | undefined; aide: string }) {
  const value = () => props.field?.value ?? "";
  const t = () => props.type.type;
  return (
    <Cases fallback={<RecordChip text={value()} />}>
      <Match when={t() === "choice" || t() === "flag"}>{choiceOf(value(), props.aide)}</Match>
      <Match when={props.type.name === "Assignee"}>
        <Show when={linkOf(value())} fallback={choiceOf(value(), props.aide)}>
          <RecordChip text={value()} />
        </Show>
      </Match>
      <Match when={t() === "date" || t() === "date-or-minute"}>{dateText(value())}</Match>
      <Match when={t() === "money"}>{moneyText(value())}</Match>
      <Match when={t() === "url"}>
        <a href={value().replace(/^<|>$/g, "")} target="_blank" rel="noreferrer">
          {value().replace(/^<|>$/g, "")}
        </a>
      </Match>
      <Match when={t() === "list" || t() === "links"}>
        <span class="chips-line">
          <For each={props.field?.values ?? []}>
            {(v) => (
              <Show when={linkOf(v)} fallback={<Tag>{v}</Tag>}>
                <RecordChip text={v} />
              </Show>
            )}
          </For>
        </span>
      </Match>
    </Cases>
  );
}

function ListEdit(props: { label: string; values: string[]; onSave: (v: FieldValue) => void }) {
  const [text, setText] = createSignal("");
  const add = () => {
    const v = text().trim();
    if (!v || props.values.includes(v)) return;
    props.onSave(many([...props.values, v]));
    setText("");
  };
  return (
    <span class="chips-edit">
      <For each={props.values}>
        {(v) => (
          <Tag class="removable">
            {v}
            <button
              type="button"
              aria-label={`${v}を外す`}
              onClick={() => props.onSave(props.values.length > 1 ? many(props.values.filter((x) => x !== v)) : one(""))}
            >
              <Icon svg={icons.x} size={12} />
            </button>
          </Tag>
        )}
      </For>
      <TextField
        aria-label={`${props.label}に足す`}
        placeholder="足す"
        value={text()}
        onInput={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.isComposing) {
            e.preventDefault();
            add();
          }
        }}
        onBlur={add}
      />
    </span>
  );
}

function LinksEdit(props: { label: string; of: string; values: string[]; single: boolean; onSave: (v: FieldValue) => void }) {
  const without = (v: string) => props.values.filter((x) => x !== v);
  const save = (values: string[]) => props.onSave(values.length === 0 ? one("") : props.single ? one(values[0]) : many(values));
  return (
    <span class="chips-edit">
      <For each={props.values}>
        {(v) => (
          <Tag class="removable">
            <RecordChip text={v} />
            <button type="button" aria-label="外す" onClick={() => save(without(v))}>
              <Icon svg={icons.x} size={12} />
            </button>
          </Tag>
        )}
      </For>
      <RecordPicker
        label={`${props.label}を選ぶ`}
        kinds={props.of ? [props.of] : undefined}
        onPick={(r) => {
          const link = linkTo(r);
          if (!props.values.includes(link)) save(props.single ? [link] : [...props.values, link]);
        }}
      />
    </span>
  );
}

function TimeEdit(props: { label: string; value: string; onSave: (v: FieldValue) => void }) {
  const day = () => props.value.slice(0, 10);
  const time = () => props.value.slice(11, 16);
  const save = (d: string, t: string) => props.onSave(one(d ? (t ? `${d} ${t}` : d) : ""));
  return (
    <span class="row">
      <TextField type="date" aria-label={`${props.label}の日付`} value={day()} onChange={(e) => save(e.currentTarget.value, time())} />
      <TextField
        type="time"
        aria-label={`${props.label}の時刻`}
        value={time()}
        disabled={!day()}
        onChange={(e) => save(day(), e.currentTarget.value)}
      />
    </span>
  );
}

function MoneyEdit(props: { label: string; value: string; onSave: (v: FieldValue) => void }) {
  const amount = () => props.value.split(" ")[0] ?? "";
  const currency = () => props.value.split(" ")[1] || "JPY";
  const save = (a: string, c: string) => props.onSave(one(a.trim() ? `${a.trim()} ${c}` : ""));
  return (
    <span class="row">
      <TextField
        inputmode="decimal"
        aria-label={`${props.label}の額`}
        value={amount()}
        onChange={(e) => save(e.currentTarget.value, currency())}
      />
      <Select aria-label={`${props.label}の通貨`} value={currency()} onChange={(e) => save(amount(), e.currentTarget.value)}>
        <For each={[...new Set(["JPY", "USD", "EUR", "GBP", currency()])]}>{(c) => <option value={c}>{c}</option>}</For>
      </Select>
    </span>
  );
}

/** The control that changes a field, by what the field holds. */
export default function FieldEdit(props: {
  type: FieldSpec;
  label: string;
  field: FieldLike | undefined;
  onSave: (v: FieldValue) => void;
}) {
  const owner = useOwner();
  const value = () => props.field?.value ?? "";
  const values = () => props.field?.values ?? [];
  const t = () => props.type.type;
  let box: HTMLInputElement | undefined;
  createEffect(() => {
    if (box && document.activeElement !== box) box.value = value();
  });
  return (
    <Cases
      fallback={
        <TextField
          class="grow"
          ref={box}
          aria-label={props.label}
          type={t() === "number" ? "number" : t() === "date" ? "date" : "text"}
          value={t() === "url" ? value().replace(/^<|>$/g, "") : value()}
          onChange={(e) => {
            const v = e.currentTarget.value.trim();
            if (v !== value()) props.onSave(one(v));
          }}
        />
      }
    >
      <Match when={t() === "choice"}>
        <Select aria-label={props.label} value={value()} onChange={(e) => props.onSave(one(e.currentTarget.value))}>
          <option value="">—</option>
          <For each={props.type.choices}>{(c) => <option value={c}>{choiceOf(c, owner.aide())}</option>}</For>
        </Select>
      </Match>
      <Match when={t() === "flag"}>
        <Switch hidden checked={value() === "yes"} onChange={(on) => props.onSave(one(on ? "yes" : "no"))}>
          {props.label}
        </Switch>
      </Match>
      <Match when={props.type.name === "Assignee"}>
        <span class="row">
          <Select
            aria-label={props.label}
            value={linkOf(value()) ? "other" : value() || OWNER}
            onChange={(e) => {
              if (e.currentTarget.value !== "other") props.onSave(one(e.currentTarget.value));
            }}
          >
            <option value={OWNER}>あなた</option>
            <option value={SECRETARY}>{owner.aide()}</option>
            <option value={WORKER}>{choiceOf(WORKER)}</option>
            <Show when={linkOf(value())}>{(l) => <option value="other">{l().title}</option>}</Show>
          </Select>
          <RecordPicker label="相手に任せる" kinds={["parties"]} onPick={(r) => props.onSave(one(linkTo(r)))} />
        </span>
      </Match>
      <Match when={t() === "date-or-minute"}>
        <TimeEdit label={props.label} value={value()} onSave={props.onSave} />
      </Match>
      <Match when={t() === "money"}>
        <MoneyEdit label={props.label} value={value()} onSave={props.onSave} />
      </Match>
      <Match when={t() === "list"}>
        <ListEdit label={props.label} values={values()} onSave={props.onSave} />
      </Match>
      <Match when={t() === "link" || t() === "links"}>
        <LinksEdit label={props.label} of={props.type.of} values={values()} single={t() === "link"} onSave={props.onSave} />
      </Match>
    </Cases>
  );
}
