import { type Client, Code, ConnectError, createClient, type Interceptor, type Transport } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import type { DescService } from "@bufbuild/protobuf";
import { MethodOptions_IdempotencyLevel as Idempotency } from "@bufbuild/protobuf/wkt";
import { isServer } from "solid-js/web";
import { CalendarService } from "~/gen/famulen/v1/calendar_pb";
import { ConsoleService } from "~/gen/famulen/v1/console_pb";
import { WorkgroupService } from "~/gen/famulen/v1/workgroups_pb";
import { MemoryService } from "~/gen/famulen/v1/memory_pb";
import { NotebookService } from "~/gen/famulen/v1/notebook_pb";
import { RecordService } from "~/gen/famulen/v1/records_pb";
import { ChannelService, ConnectionService } from "~/gen/famulen/v1/services_pb";
import {
  ModelService,
  LogService,
  RoutineService,
  SettingsService,
} from "~/gen/famulen/v1/settings_pb";
import { GatewayLogService, VaultService } from "~/gen/famulen/v1/vault_pb";
import { LookoutService } from "~/gen/famulen/v1/lookout_pb";
import { WorkerService } from "~/gen/famulen/v1/workers_pb";

/** Where the backend answers, as the console's server reaches it. */
export function backendUrl(): string {
  return process.env.FAMULEN_API_URL ?? "http://127.0.0.1:7790";
}

/** How long a read waits for a backend that is starting again. */
const AWAY_MS = 30_000;

const away = (e: unknown) =>
  e instanceof ConnectError && (e.code === Code.Unavailable || e.cause instanceof TypeError);

/**
 * Famulen starts again by itself, such as to move the notebook. Reads wait
 * for it, so the page on screen stays; changes are never sent twice.
 */
const waitForBackend: Interceptor = (next) => async (req) => {
  const until = Date.now() + AWAY_MS;
  for (;;) {
    try {
      return await next(req);
    } catch (e) {
      const read = !req.stream && req.method.idempotency === Idempotency.NO_SIDE_EFFECTS;
      if (!read || !away(e) || req.signal.aborted || Date.now() > until) throw e;
      await new Promise((r) => setTimeout(r, 500));
    }
  }
};

let browserTransport: Transport | undefined;

/**
 * The browser calls the console's own origin, which relays to the backend;
 * the server rendering a page calls the backend directly.
 */
function transport(): Transport {
  if (isServer) return createConnectTransport({ baseUrl: backendUrl() });
  browserTransport ??= createConnectTransport({ baseUrl: location.origin, interceptors: [waitForBackend] });
  return browserTransport;
}

/** Picks its transport per call, so one client serves both sides. */
const lazy: Transport = {
  unary: (...args) => transport().unary(...args),
  stream: (...args) => transport().stream(...args),
};

function client<T extends DescService>(service: T): Client<T> {
  return createClient(service, lazy);
}

export const api = {
  notebook: client(NotebookService),
  records: client(RecordService),
  console: client(ConsoleService),
  calendar: client(CalendarService),
  workgroups: client(WorkgroupService),
  settings: client(SettingsService),
  models: client(ModelService),
  routines: client(RoutineService),
  log: client(LogService),
  connections: client(ConnectionService),
  channels: client(ChannelService),
  lookout: client(LookoutService),
  memory: client(MemoryService),
  vault: client(VaultService),
  gatewayLog: client(GatewayLogService),
  workers: client(WorkerService),
};
