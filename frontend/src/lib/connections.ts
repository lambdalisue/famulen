import { query } from "@solidjs/router";
import { api } from "./api";
import * as icons from "./icons";

export const lookoutChoicesQuery = query((id: string) => api.lookout.getLookoutChoices({ id }), "lookout-choices");
export const connectionsQuery = query(() => api.connections.listConnections({}), "connections");
export const connectionQuery = query((id: string) => api.connections.getConnection({ id }), "connection");
export const kindQuery = query((kind: string) => api.connections.getKind({ id: kind }), "kind");
export const knownQuery = query(async () => (await api.connections.listKnown({})).services, "known");
export const workspacesQuery = query(() => api.channels.listWorkspaces({}), "workspaces");
export const channelsQuery = query(async () => (await api.channels.listChannels({})).channels, "channels");
export const definitionsQuery = query(() => api.connections.listDefinitions({}), "definitions");
export const builtinToolsQuery = query(async () => (await api.connections.listBuiltinTools({})).tools, "builtin-tools");
export const definitionChangesQuery = query(
  async () => (await api.connections.listDefinitionChanges({ definition: "" })).changes,
  "definition-changes",
);

/** What each of Famulen's destinations is called in the console. */
export const DESTINATION_LABEL: Record<string, string> = {
  channel: "チャネル",
  lookout: "見張り",
  calendar: "カレンダー",
  secretary: "秘書",
};

/** Where a setting the console cannot make is explained, by the doc's path. */
export function docUrl(path: string): string {
  return `https://github.com/lambdalisue/famulen/blob/main/${path}`;
}

/** The setup doc an `unavailable` text names, if any. */
export function docIn(text: string): string | undefined {
  return /(?:docs|crates)\/[\w./-]+\.md/.exec(text)?.[0];
}

const BY_NAME: Record<string, string> = {
  "brand-google": icons.brandGoogle,
  "brand-notion": icons.brandNotion,
  "brand-adroit": icons.terminal,
  "brand-bluesky": icons.brandBluesky,
  "brand-discord": icons.brandDiscord,
  "brand-github": icons.brandGithub,
  "brand-asana": icons.brandAsana,
  "brand-jira": icons.brandJira,
  "brand-kingoftime": icons.clock,
  "brand-kintone": icons.table,
  "brand-windows": icons.brandWindows,
  "brand-slack": icons.brandSlack,
  "list-check": icons.listCheck,
  "list-details": icons.listDetails,
  calculator: icons.calculator,
  check: icons.check,
  notes: icons.notes,
};

/** A known service's icon, from the Tabler name the API gives. */
export function iconNamed(name: string): string {
  return BY_NAME[name] ?? icons.plugConnected;
}
