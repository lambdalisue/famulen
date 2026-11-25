import { createContext, type JSX, useContext } from "solid-js";
import type { Persona } from "~/gen/famulen/v1/settings_pb";
import { personaQuery } from "./data";
import { today } from "./dates";
import { createKept } from "./kept";
import { portraitOf } from "./portrait";

/** What the owner calls the secretary until the owner names it. */
const UNNAMED = "秘書";

const Read = createContext<() => Persona | undefined>();

/**
 * Reads it once for all that is inside. Read where it is used, each element
 * put on the screen later, such as the icon by a new message, would wait for a
 * reading of its own, and have the Suspense around the whole console take
 * everything off the screen meanwhile: the focus, the keyboard and where things
 * are scrolled to go with it.
 */
export function OwnerProvider(props: { children?: JSX.Element }) {
  const persona = createKept(() => personaQuery());
  return <Read.Provider value={() => persona.latest}>{props.children}</Read.Provider>;
}

/** What the owner decided that every page reads: the secretary's name and portrait and the time zone days are counted in. */
export function useOwner() {
  const persona = useContext(Read);
  if (!persona) throw new Error("useOwner needs an OwnerProvider around it");
  const tz = () => persona()?.timezone || "UTC";
  return {
    /** The secretary's name. */
    aide: () => persona()?.name || UNNAMED,
    /** Which of the two portraits stands for the secretary. */
    portrait: () => portraitOf(persona()?.gender),
    tz,
    /** Today as YYYY-MM-DD where the owner is. */
    today: () => today(tz()),
    persona,
  };
}
