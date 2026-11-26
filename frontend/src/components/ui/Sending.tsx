import { type ComponentProps, createContext, type JSX, splitProps, useContext } from "solid-js";
import { createSending, type Sending } from "~/lib/sending";

/** What sends together; a form's is shared only by what submits it. */
type Group = { sending: Sending; hold: boolean; form?: boolean };

const Shared = createContext<Group>();

/**
 * The sending the buttons around share, or one of a button's own. Within a
 * form only what submits it shares the form's; what else it holds, a folder
 * picker or a lookup, sends apart.
 */
export function useSending(submits = false): Group {
  const shared = useContext(Shared);
  return shared && (!shared.form || submits) ? shared : { sending: createSending(), hold: false };
}

/**
 * Buttons that act on the same thing, such as approving and denying one
 * request: while one of them sends, none of them can be pressed. With `hold`
 * they stay so once it is done, for what they act on goes away.
 */
export function SendTogether(props: { hold?: boolean; sending?: Sending; children: JSX.Element }) {
  const group: Group = { sending: props.sending ?? createSending(), hold: props.hold ?? false };
  return <Shared.Provider value={group}>{props.children}</Shared.Provider>;
}

type Submitted = SubmitEvent & { currentTarget: HTMLFormElement; target: Element };

/**
 * A form sent once at a time, by its button or by Enter. Its submitting
 * buttons wait while it sends; what is written in it stays open to change.
 */
export function Form(props: Omit<ComponentProps<"form">, "onSubmit"> & { onSubmit: (e: Submitted) => unknown; hold?: boolean }) {
  const [own, rest] = splitProps(props, ["onSubmit", "hold", "children"]);
  const group: Group = { sending: createSending(), hold: own.hold ?? false, form: true };
  return (
    <form
      {...rest}
      aria-busy={group.sending.busy() || undefined}
      onSubmit={(e) => {
        e.preventDefault();
        group.sending.run(() => own.onSubmit(e), group.hold);
      }}
    >
      <Shared.Provider value={group}>{own.children}</Shared.Provider>
    </form>
  );
}
