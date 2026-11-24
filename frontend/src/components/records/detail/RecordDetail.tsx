import { createAsync, revalidate, useNavigate } from "@solidjs/router";
import { createEffect, createSignal, ErrorBoundary, For, type JSX, Show, Suspense } from "solid-js";
import { Button, EmptyState, ErrorState, Keyed, Loading, whenConfirmed } from "~/components/ui";
import type { RecordKind, SectionType } from "~/gen/famulen/v1/records_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { dateText } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import {
  fieldOf,
  KIND_NAMES,
  kindsQuery,
  nameOf,
  plainField,
  plainItem,
  type Rec,
  recordQuery,
  RELATED,
  same,
  sectionIn,
  withField,
} from "~/lib/records";
import { setSelection } from "~/lib/view";
import Backlinks from "../links/Backlinks";
import { KnownLinks } from "../links/known";
import RelatedLinks from "../links/RelatedLinks";
import Details from "./Details";
import Fields from "./Fields";
import Header, { copy } from "./Header";
import Items from "./Items";
import Log from "./Log";
import Notes from "./Notes";
import Standing from "./Standing";
import { type SaveRecord, useRecordEdit } from "./useRecordEdit";

type Extra = {
  /** Where to go once it is removed. */
  back: string;
  /** What this kind shows above the rest. */
  top?: (r: Rec) => JSX.Element;
  /** What this kind shows below its fields. */
  extra?: (r: Rec) => JSX.Element;
};

/** The record's file, as it is written. */
function Raw(props: { record: Rec }) {
  return (
    <section class="rec-section">
      <header class="rec-head">
        <span class="sub mono grow">
          records/{props.record.kind}/{props.record.id}.md
        </span>
        <Button small icon={icons.copy} onClick={() => copy(props.record.markdown, "原文")}>
          写す
        </Button>
      </header>
      <pre class="rec-raw">{props.record.markdown}</pre>
    </section>
  );
}

/** The sections of a kind that are filled in, and those that can be. */
function Sections(props: { record: Rec; kind: RecordKind; aide: string; save: SaveRecord }) {
  const [added, setAdded] = createSignal<string[]>([]);
  const given = (s: SectionType) => sectionIn(props.record, s.name);
  const shown = () => props.kind.sections.filter((s) => given(s) || added().includes(s.name));
  const rest = () => props.kind.sections.filter((s) => !given(s) && !added().includes(s.name));
  return (
    <>
      <Keyed each={shown()} by={(s) => s.name}>
        {(s) => (
          <section class="rec-section">
            <h3>{nameOf(s().name)}</h3>
            <Show
              when={s().many}
              fallback={
                <Fields
                  types={s().fields}
                  fields={given(s())?.fields ?? []}
                  aide={props.aide}
                  onSave={(name, v) =>
                    void props.save((now) => ({
                      sections: [{ name: s().name, fields: withField(sectionIn(now, s().name)?.fields ?? [], name, v) }],
                    }))
                  }
                />
              }
            >
              <Items
                type={s()}
                items={given(s())?.items ?? []}
                aide={props.aide}
                onSave={(change) =>
                  void props.save((now) => ({
                    sections: [{ name: s().name, items: change((sectionIn(now, s().name)?.items ?? []).map(plainItem)) }],
                  }))
                }
              />
            </Show>
          </section>
        )}
      </Keyed>
      <Show when={rest().length > 0}>
        <div class="chips-line">
          <For each={rest()}>
            {(s) => (
              <button type="button" class="tag add" onClick={() => setAdded([...added(), s.name])}>
                ＋ {nameOf(s.name)}
              </button>
            )}
          </For>
        </div>
      </Show>
    </>
  );
}

function Content(props: { record: Rec } & Extra) {
  const navigate = useNavigate();
  const owner = useOwner();
  const r = () => props.record;
  const kinds = createAsync(() => kindsQuery());
  const kind = () => kinds()?.find((k) => k.name === r().kind);
  /** A kind with a text of its own instead of fields, notes and a log. */
  const free = () => kind()?.free ?? false;
  const [view, setView] = createSignal("view");
  const save = useRecordEdit({ kind: props.record.kind, id: props.record.id });
  createEffect(() => setSelection(`${KIND_NAMES[r().kind] ?? r().kind}：${r().title}（${r().code}）`));

  const remove = () =>
    whenConfirmed(`「${r().title}」を消す`, "消す", async () => {
      const { kind, id } = r();
      if (await act(() => api.records.deleteRecord({ kind, id }), "消した")) navigate(props.back);
    });

  return (
    <KnownLinks.Provider value={() => r().links}>
      <article class="rec">
        <Header record={r()} view={view()} onView={setView} save={save} onRemove={() => void remove()} />
        <Show when={view() === "view"} fallback={<Raw record={r()} />}>
          <Standing record={r()} />
          {props.top?.(r())}
          <div class="rec-columns">
            <div class="rec-body">
              <Show when={!free() && kind()}>
                {(k) => (
                  <section class="rec-section rec-glance" aria-label="項目">
                    <Fields
                      types={k().fields}
                      fields={r().fields}
                      aide={owner.aide()}
                      skip={[RELATED]}
                      onSave={(name, v) => void save({ fields: [{ name, ...v }] })}
                    />
                  </section>
                )}
              </Show>
              <Notes
                text={r().notes}
                label={free() ? "本文" : "備考"}
                empty={free() ? "まだ何も書いていない" : "備考はまだない"}
                links={r().links}
                onSave={async (notes) => (await save({ notes })) !== undefined}
              />
              {props.extra?.(r())}
            </div>
            <div class="rec-aside">
              <Show when={kind()}>{(k) => <Sections record={r()} kind={k()} aide={owner.aide()} save={save} />}</Show>
              <RelatedLinks
                record={r()}
                onSave={(change) =>
                  void save((now) => ({ fields: [{ name: RELATED, value: "", values: change(fieldOf(now, RELATED)?.values ?? []) }] }))
                }
              />
              <Backlinks record={r()} />
              <Show when={!free()}>
                <Details
                  fields={r().details}
                  onSave={(change) => void save((now) => ({ details: change(now.details.map(plainField)), setDetails: true }))}
                />
              </Show>
            </div>
            <Show when={!free()}>
              <div class="rec-log-column">
                <Log record={r()} aide={owner.aide()} onAdd={async (line) => (await save({ log: [line] }, "書いた")) !== undefined} />
              </div>
            </Show>
          </div>
          <p class="sub rec-foot">
            <Show when={r().source}>
              <span>{r().source}</span>
            </Show>
            <Show when={r().created}>
              <span>作った {dateText(r().created.slice(0, 16))}</span>
            </Show>
            <Show when={r().updated}>
              <span>直した {dateText(r().updated.slice(0, 16))}</span>
            </Show>
          </p>
        </Show>
      </article>
    </KnownLinks.Provider>
  );
}

/** Everything one record says, each part changed where it stands. */
export default function RecordDetail(props: { kind: string; id: string } & Extra) {
  const record = createAsync(() => recordQuery(props.kind, props.id));
  const there = () => record() ?? undefined;
  return (
    <ErrorBoundary
      fallback={(error, reset) => (
        <ErrorState
          error={error}
          onRetry={() => {
            void revalidate(recordQuery.keyFor(props.kind, props.id)).then(reset);
          }}
        />
      )}
    >
      <Suspense fallback={<Loading />}>
        <Show when={record() !== null} fallback={<EmptyState>この記録はもうない</EmptyState>}>
          <Show when={there()}>{(r) => <Content record={r()} back={props.back} top={props.top} extra={props.extra} />}</Show>
        </Show>
      </Suspense>
    </ErrorBoundary>
  );
}
