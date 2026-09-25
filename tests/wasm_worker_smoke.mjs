import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { once } from "node:events";

import { createWorkerObjectStore } from "../src/worker-object-store.mjs";
import { createWorkerQueryStream } from "../src/worker-query-stream.mjs";
import { createRequire } from "node:module";
import path from "node:path";

const packageDirectory = path.resolve(process.argv[2] ?? "target/wasm-bindgen");
const require = createRequire(import.meta.url);
const { WasmObjectTable } = require(path.join(packageDirectory, "txbase.js"));

const xbf = Uint8Array.from(
  Buffer.from(
    "VFhCRgEAAAAAAAAAZAAAAGQAAAAAAAAAKwAAAAAAAADx2z6tjwAAAAAAAAAwAAAAAAAAAJs4vMK/AAAAAAAAAFIAAAAAAAAAiF8HnAIAAAAAAAAAAAAAAAAAAAB76dDSAAAAAAQAAAACAElEEQAAAAQATkFNRTAAAAADAEFHRREAAAAGAEFDVElWRQEAAAC/AAAAAAAAACoAAAAAAAAAAAAAAAAAAADpAAAAAAAAACgAAAAAAAAAAQAAAAAAAAARCAAAAAEAAAAAAAAAMAUAAABBbGljZREIAAAAHQAAAAAAAAABAQAAAAERCAAAAAIAAAAAAAAAMAMAAABCb2IRCAAAAAcAAAAAAAAAAQEAAAAA",
    "base64",
  ),
);

const objects = new Map();
const slowKeys = new Set(["slow"]);

const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, "http://127.0.0.1");
    if (request.method === "GET" && url.pathname === "/objects/") {
      const prefix = url.searchParams.get("prefix");
      if (prefix === null) return send(response, 400, "missing prefix");
      return send(
        response,
        200,
        JSON.stringify([...objects.keys()].filter((key) => key.startsWith(prefix))),
        { "content-type": "application/json" },
      );
    }
    if (!url.pathname.startsWith("/objects/")) return send(response, 404);
    const key = url.pathname
      .slice("/objects/".length)
      .split("/")
      .map(decodeURIComponent)
      .join("/");
    if (slowKeys.has(key)) await delay(60);

    if (request.method === "GET") {
      const bytes = objects.get(key);
      return bytes === undefined
        ? send(response, 404)
        : send(response, 200, Buffer.from(bytes), { etag: etag(bytes) });
    }
    if (request.method === "PUT") {
      const current = objects.get(key);
      if (request.headers["if-none-match"] === "*" && current !== undefined) {
        return send(response, 412, "already exists");
      }
      if (
        request.headers["if-match"] !== undefined &&
        (current === undefined || request.headers["if-match"] !== etag(current))
      ) {
        return send(response, 412, "precondition failed");
      }
      objects.set(key, await bodyBytes(request));
      return send(response, 204);
    }
    if (request.method === "DELETE") {
      objects.delete(key);
      return send(response, 204);
    }
    return send(response, 405);
  } catch (error) {
    send(response, 500, error instanceof Error ? error.message : String(error));
  }
});

try {
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const { port } = server.address();
  const baseUrl = `http://127.0.0.1:${port}/objects/`;
  const store = createWorkerObjectStore({ baseUrl, timeoutMs: 1_000 });
  const table = new WasmObjectTable(store, "users");
  assert.throws(
    () => createWorkerObjectStore({ baseUrl, signal: {} }),
    /AbortSignal/,
  );
  await assert.rejects(
    () => store.get("users/../secret"),
    /ordinary non-empty path components/,
  );

  assert.equal(await table.manifest_json(), null);
  assert.equal(await table.read_xbf(), null);
  const committed = JSON.parse(await table.commit_xbf(xbf));
  assert.deepEqual(committed, { status: "committed", generation: 0 });
  assert.deepEqual([...await table.read_xbf()], [...xbf]);
  assert.deepEqual(
    (await store.list("users/")).sort(),
    [
      "users/manifest.json",
      "users/pages/0/0.bin",
      "users/snapshots/0.pages.json",
    ],
  );
  assert.equal(await store.get("users/missing.xbf"), null);
  await assert.rejects(
    () => store.compareAndSwap("users/manifest.json", Uint8Array.of(1), xbf),
    (error) => error.code === "conflict",
  );

  const timeoutStore = createWorkerObjectStore({ baseUrl, timeoutMs: 5 });
  await assert.rejects(
    () => timeoutStore.get("slow"),
    (error) => error.code === "unavailable" && error.message.includes("timed out"),
  );

  const controller = new AbortController();
  const cancelledStore = createWorkerObjectStore({
    baseUrl,
    signal: controller.signal,
    timeoutMs: 1_000,
  });
  const pending = cancelledStore.get("slow");
  setTimeout(() => controller.abort("client disconnected"), 1);
  await assert.rejects(
    pending,
    (error) => error.code === "cancelled" && error.message.includes("client disconnected"),
  );
  const cancelledTable = new WasmObjectTable(cancelledStore, "users");
  await assert.rejects(() => cancelledTable.read_xbf(), /cancelled/);

  const querySignals = [];
  const startedResolvers = [];
  const queryReadsStarted = [0, 1].map(
    () => new Promise((resolve) => startedResolvers.push(resolve)),
  );
  const queryStore = createWorkerObjectStore({
    baseUrl,
    timeoutMs: 1_000,
    fetchImpl(url, init) {
      if (
        new URL(url).pathname === "/objects/users/manifest.json" &&
        querySignals.length < 2
      ) {
        const index = querySignals.length;
        querySignals.push(init.signal);
        startedResolvers[index]();
        return new Response(
          new ReadableStream({
            start(controller) {
              init.signal.addEventListener(
                "abort",
                () => controller.error(init.signal.reason),
                { once: true },
              );
              if (init.signal.aborted) controller.error(init.signal.reason);
            },
          }),
          { status: 200 },
        );
      }
      return fetch(url, init);
    },
  });
  const queryTable = new WasmObjectTable(queryStore, "users");
  const firstReader = createWorkerQueryStream({ database: queryTable }).getReader();
  const firstRead = firstReader.read();
  await queryReadsStarted[0];
  const queryController = new AbortController();
  const secondReader = createWorkerQueryStream({
    database: queryTable,
    signal: queryController.signal,
  }).getReader();
  const secondRead = secondReader.read();
  await queryReadsStarted[1];

  await firstReader.cancel("reader disconnected");
  assert.equal(querySignals[0].aborted, true);
  assert.equal(querySignals[1].aborted, false);
  queryController.abort("client disconnected");
  await assert.rejects(
    secondRead,
    (error) => error.code === "cancelled" && error.message.includes("client disconnected"),
  );
  assert.equal(querySignals[1].aborted, true);
  assert.equal((await firstRead).done, true);
} finally {
  server.close();
  await once(server, "close");
}

console.log("wasm worker object-store smoke passed");

function etag(bytes) {
  return `"${createHash("sha256").update(bytes).digest("base64url")}"`;
}

function send(response, status, body = "", headers = {}) {
  response.writeHead(status, headers);
  response.end(body);
}

async function bodyBytes(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return Uint8Array.from(Buffer.concat(chunks));
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
