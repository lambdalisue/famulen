import { createSignal } from "solid-js";
import * as icons from "~/lib/icons";
import { linkTo, type Rec } from "~/lib/records";
import { PickDialog } from "../RecordPicker";
import { Button } from "~/components/ui";

/** Markdown being written, where `[[` or the button picks a record to link. */
export default function LinkTextarea(props: { label: string; value: string; onInput: (text: string) => void; onSave: () => void }) {
  let box!: HTMLTextAreaElement;
  const [open, setOpen] = createSignal(false);
  /** What the link will stand in place of. */
  let place: [number, number] = [0, 0];
  const start = (from: number, to: number) => {
    place = [from, to];
    setOpen(true);
  };
  const insert = (r: Rec) => {
    const link = linkTo(r);
    box.setRangeText(link, place[0], place[1], "end");
    props.onInput(box.value);
  };
  return (
    <>
      <div class="row">
        <Button small icon={icons.plus} onClick={() => start(box.selectionStart, box.selectionEnd)}>
記録へのリンク
        </Button>
        <span class="sub">[[ でも選べる</span>
      </div>
      <textarea
        ref={box}
        class="input rec-notes"
        aria-label={props.label}
        placeholder="Markdown で書ける"
        value={props.value}
        onInput={(e) => {
          props.onInput(e.currentTarget.value);
          const at = box.selectionStart;
          if (e.data?.endsWith("[") && !e.isComposing && /(^|[^[])\[\[$/.test(box.value.slice(Math.max(0, at - 3), at))) start(at - 2, at);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) props.onSave();
        }}
      />
      <PickDialog label="記録へのリンク" open={open()} onOpenChange={setOpen} onPick={insert} focusAfter={() => box} />
    </>
  );
}
