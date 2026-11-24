/** Start a request in the conversation box with `text`, ready to finish. */
export function ask(text: string) {
  const say = document.getElementById("say");
  if (!(say instanceof HTMLTextAreaElement)) return;
  say.value = text;
  // The box keeps its text in a signal fed by input events.
  say.dispatchEvent(new Event("input", { bubbles: true }));
  say.focus();
  say.setSelectionRange(text.length, text.length);
}

/** Pull the conversation up in the narrow layout. */
export function openSheet() {
  const peek = document.querySelector(".m-peek");
  if (peek instanceof HTMLButtonElement && getComputedStyle(peek).display !== "none") peek.click();
  else document.getElementById("say")?.focus();
}
