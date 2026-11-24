import { createAsync } from "@solidjs/router";
import { personaQuery } from "./data";
import { today } from "./dates";
import { portraitOf } from "./portrait";

/** What the owner calls the secretary until the owner names it. */
const UNNAMED = "秘書";

/** What the owner decided that every page reads: the secretary's name and portrait and the time zone days are counted in. */
export function useOwner() {
  const persona = createAsync(() => personaQuery());
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
