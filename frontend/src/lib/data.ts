import { query } from "@solidjs/router";
import { api } from "./api";

export const workgroupsQuery = query(async () => (await api.workgroups.listWorkgroups({})).workgroups, "workgroups");
export const inboxQuery = query(() => api.console.getInbox({}), "inbox");
export const statusQuery = query(() => api.console.getStatus({}), "status");
export const personaQuery = query(() => api.settings.getPersona({}), "persona");
export const writableQuery = query(async () => (await api.settings.getWritablePlaces({})).paths, "writable");
export const notebookPlaceQuery = query(() => api.settings.getNotebookPlace({}), "notebookPlace");
export const notebookSharingQuery = query(() => api.settings.getNotebookSharing({}), "notebookSharing");
export const slackNamesQuery = query(async () => (await api.channels.listSlackNames({})).names, "slackNames");
export const messagesQuery = query(
  async (workgroup: string) => (await api.console.listMessages({ workgroup, limit: 120 })).messages,
  "messages",
);
/** The home of the user Famulen runs as, written in full; `~/` in a path the owner writes is read from it. */
export const homeQuery = query(async () => (await api.settings.getFolder({ path: "" })).path, "home");
