/*
 * The console's parts. A page is put together from these and adds only the
 * layout of its own.
 *
 * Ark UI holds what a browser has no control of its own for (dialogs,
 * segments, switches, number fields, disclosures); a text box, a check box and
 * a select are the browser's, styled.
 *
 * A part asks for what it is given once (`"note" in props`, not `props.note`,
 * to tell whether it is): elements are made anew each time they are asked for,
 * and would not match what the server sent.
 *
 * How things are saved:
 * - A choice, a switch, a number or a short text is saved as it is made: when
 *   it is chosen, or when its field is left or Enter is pressed.
 * - Long text, and whatever is put in a dialog or goes into the vault, is
 *   saved with a button; leaving it half written asks first.
 * - Removing asks first with `confirmed`, unless one value put back by hand
 *   undoes it.
 * - `act` says how a change went; a read that fails shows `ErrorState`.
 * - What a press or a form sends goes once: a button whose press gives back a
 *   promise cannot be pressed until it is back, nor can those it is put
 *   `SendTogether` with; a `Form` is sent once at a time.
 */
export { BackLink, Button, buttonClass, type ButtonVariant, createPress, IconButton, LinkButton } from "./Button";
export { confirmed, ConfirmHost, whenConfirmed } from "./Confirm";
export { Dialog, DialogActions, DialogTrigger } from "./Dialog";
export { Callout, EmptyState, ErrorState, Loading, Meter, Tag, type Tone } from "./feedback";
export { default as Icon } from "./Icon";
export { Checkbox, Field, NumberField, SearchBox, Select, Switch, TextArea, TextField } from "./inputs";
export { default as Keyed } from "./Keyed";
export { Disclosure, Legend, PageHeader, Pager, Section, SubSection } from "./layout";
export { List, ListRow } from "./List";
export { RadioCards } from "./RadioCards";
export { Segmented, SegmentedLinks, SegmentedOf } from "./Segmented";
export { Form, SendTogether, useSending } from "./Sending";
export { default as SideSheet } from "./SideSheet";
export { default as SplitPane } from "./SplitPane";
export { default as Toast } from "./Toast";
