/** A line of plain text, with Slack's marks as Markdown's. */
function inline(text: string): string {
  return text
    .split(/(`[^`\n]*`)/)
    .map((part, i) =>
      i % 2 === 1
        ? part
        : part
            .replace(/<((?:https?|mailto):[^|>\s]+)\|([^>\n]+)>/g, "[$2]($1)")
            .replace(/(^|[^*\w])\*([^*\s](?:[^*\n]*[^*\s])?)\*(?![*\w])/g, "$1**$2**")
            .replace(/(^|[^~\w])~([^~\s](?:[^~\n]*[^~\s])?)~(?![~\w])/g, "$1~~$2~~"),
    )
    .join("");
}

/**
 * What is said in the conversation, as Markdown that is drawn the way a chat
 * shows it: a line ends where it was ended, and what Slack's own marks make
 * bold, struck or a link is so here too.
 */
export function chatMarkdown(text: string): string {
  let fence = "";
  return text
    .split("\n")
    .map((line) => {
      const mark = /^\s*(`{3,}|~{3,})/.exec(line)?.[1] ?? "";
      if (fence) {
        if (mark.startsWith(fence)) fence = "";
        return line;
      }
      if (mark) {
        fence = mark;
        return line;
      }
      if (!line.trim()) return line;
      const said = inline(line.replace(/^(\s*)• /, "$1- "));
      // Two spaces at its end are a line break in Markdown.
      return `${said.trimEnd()}  `;
    })
    .join("\n");
}
