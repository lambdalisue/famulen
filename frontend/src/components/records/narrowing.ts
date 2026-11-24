import { createMemo, createSignal } from "solid-js";
import { matches, type Rec, valueOf } from "~/lib/records";

/** What a list is narrowed by: words, labels, choices, what is over, what may be out of date. */
export function createNarrowing(of: { all: () => Rec[]; over?: (r: Rec) => boolean }) {
  const [words, setWords] = createSignal("");
  const [labels, setLabels] = createSignal<string[]>([]);
  const [chosen, setChosen] = createSignal<{ [field: string]: string }>({});
  const [withOver, setWithOver] = createSignal(false);
  const [onlyDoubted, setOnlyDoubted] = createSignal(false);
  const anyChosen = () => Object.values(chosen()).some((v) => v);
  return {
    words,
    setWords,
    chosen,
    choose: (field: string, value: string) => setChosen({ ...chosen(), [field]: value }),
    labels,
    toggleLabel: (l: string) => setLabels(labels().includes(l) ? labels().filter((x) => x !== l) : [...labels(), l]),
    withOver,
    setWithOver,
    onlyDoubted,
    setOnlyDoubted,
    /** Whether anything at all narrows the list. */
    any: () => !!words().trim() || labels().length > 0 || anyChosen() || onlyDoubted(),
    clear: () => {
      setWords("");
      setLabels([]);
      setChosen({});
      setOnlyDoubted(false);
    },
    allLabels: createMemo(() => [...new Set(of.all().flatMap((r) => r.labels))].sort()),
    anyDoubted: createMemo(() => onlyDoubted() || of.all().some((r) => r.obsolete)),
    found: createMemo(() =>
      of.all().filter(
        (r) =>
          (withOver() || !of.over?.(r) || anyChosen()) &&
          (!onlyDoubted() || !!r.obsolete) &&
          (!words().trim() || matches(r, words())) &&
          labels().every((l) => r.labels.includes(l)) &&
          Object.entries(chosen()).every(([f, v]) => !v || valueOf(r, f) === v),
      ),
    ),
  };
}
