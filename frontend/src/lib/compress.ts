import { constants, createGzip } from "node:zlib";

/** What is worth compressing: text and API messages, not pictures, which are compressed already. */
const COMPRESSIBLE = /^(text\/|application\/(json|proto|connect\+json|connect\+proto|javascript|xml)|image\/svg\+xml)/;

/** Below this, the gzip header costs about what it saves. */
const SMALL = 1024;

/** Whether the browser takes gzip. */
function takesGzip(accept: string | null): boolean {
  return (accept ?? "")
    .split(",")
    .map((e) => e.split(";").map((p) => p.trim()))
    .some(([name, ...params]) => name.toLowerCase() === "gzip" && !params.some((p) => /^q=0(\.0*)?$/.test(p)));
}

/** Gzip that sends on what each chunk gives at once, rather than when its window fills. */
function gzipEach(): TransformStream<Uint8Array, Uint8Array> {
  const gzip = createGzip();
  // Once the browser has gone away or all is sent, what gzip still gives has nowhere to go.
  let open = true;
  // What a chunk waiting on gzip is told when gzip fails.
  let failed: (e: unknown) => void = () => {};
  const close = () => {
    open = false;
    gzip.destroy();
  };
  return new TransformStream({
    start(out) {
      gzip.on("data", (c: Buffer) => {
        if (!open) return;
        try {
          out.enqueue(new Uint8Array(c));
        } catch {
          // The browser went away before the stream was told so.
          close();
        }
      });
      gzip.on("error", (e) => {
        if (open) out.error(e);
        open = false;
        failed(e);
      });
    },
    transform: (chunk) =>
      new Promise((done, fail) => {
        failed = fail;
        gzip.write(chunk);
        gzip.flush(constants.Z_SYNC_FLUSH, () => done());
      }),
    flush: () =>
      new Promise((done, fail) => {
        failed = fail;
        gzip.once("end", () => {
          open = false;
          done();
        });
        gzip.end();
      }),
  });
}

/**
 * A response compressed with gzip, when the browser takes it and the body is
 * worth it. Each chunk is flushed as it comes, so a page whose head goes out
 * first and a stream of events still arrive as they are written. A part of a
 * body asked for by range is sent as it is, as its range counts the bytes
 * before compression.
 */
export function compressed(response: Response, accept: string | null): Response {
  const length = Number(response.headers.get("content-length") ?? Number.NaN);
  if (
    !response.body ||
    !takesGzip(accept) ||
    response.status === 206 ||
    response.headers.has("content-range") ||
    response.headers.has("content-encoding") ||
    !COMPRESSIBLE.test(response.headers.get("content-type") ?? "") ||
    length < SMALL
  ) {
    return response;
  }
  const headers = new Headers(response.headers);
  headers.set("content-encoding", "gzip");
  headers.delete("content-length");
  headers.append("vary", "accept-encoding");
  return new Response(response.body.pipeThrough(gzipEach()), { status: response.status, statusText: response.statusText, headers });
}
