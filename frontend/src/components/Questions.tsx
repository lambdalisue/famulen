import { createSignal, For, Show } from "solid-js";
import type { ApprovalQuestion } from "~/gen/famulen/v1/console_pb";
import { Button, TextField } from "~/components/ui";

/** Questions the owner answers by choosing or in their own words. */
export default function Questions(props: { questions: ApprovalQuestion[]; onAnswer: (replies: string[]) => void }) {
  const [chosen, setChosen] = createSignal<string[][]>([]);
  const [words, setWords] = createSignal<string[]>([]);
  const quick = () => props.questions.length === 1 && !props.questions[0]?.several;
  const put = <T,>(all: T[], i: number, value: T) => {
    const next = [...all];
    next[i] = value;
    return next;
  };
  const reply = (i: number) => words()[i]?.trim() || (chosen()[i] ?? []).join(", ");
  const replies = () => props.questions.map((_, i) => reply(i));
  const choose = (i: number, q: ApprovalQuestion, label: string) => {
    if (quick()) {
      props.onAnswer([label]);
      return;
    }
    const now = chosen()[i] ?? [];
    const next = !q.several ? [label] : now.includes(label) ? now.filter((l) => l !== label) : [...now, label];
    setChosen((all) => put(all, i, next));
  };
  return (
    <form
      class="questions"
      onSubmit={(e) => {
        e.preventDefault();
        if (replies().every((r) => r)) props.onAnswer(replies());
      }}
    >
      <For each={props.questions}>
        {(q, i) => (
          <div class="question">
            <b>{q.question}</b>
            <div class="question-choices">
              <For each={q.choices}>
                {(c) => (
                  <Button
                    small
                    classList={{ primary: (chosen()[i()] ?? []).includes(c.label) }}
                    aria-pressed={quick() ? undefined : (chosen()[i()] ?? []).includes(c.label)}
                    title={c.description}
                    onClick={() => choose(i(), q, c.label)}
                  >
                    {c.label}
                  </Button>
                )}
              </For>
            </div>
            <For each={q.choices.filter((c) => c.description)}>
              {(c) => (
                <span class="sub">
                  {c.label}：{c.description}
                </span>
              )}
            </For>
            <TextField
              aria-label={`${q.question} に自分の言葉で答える`}
              placeholder="自分の言葉で答える"
              value={words()[i()] ?? ""}
              onInput={(e) => {
                const text = e.currentTarget.value;
                setWords((all) => put(all, i(), text));
              }}
            />
          </div>
        )}
      </For>
      <Show when={!quick() || words()[0]?.trim()}>
        <Button variant="primary" small type="submit" disabled={!replies().every((r) => r)}>
          答える
        </Button>
      </Show>
    </form>
  );
}
