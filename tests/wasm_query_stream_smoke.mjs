import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

import {
  WorkerQueryStreamError,
  createWorkerQueryStream,
} from "../src/worker-query-stream.mjs";

const packageDirectory = path.resolve(process.argv[2] ?? "target/wasm-bindgen");
const require = createRequire(import.meta.url);
const { WasmDatabase } = require(path.join(packageDirectory, "txbase.js"));

function fixture() {
  const hex = fs
    .readFileSync(new URL("./fixtures/users.dbf.hex", import.meta.url), "utf8")
    .trim()
    .split(/\s+/);
  return Uint8Array.from(hex, (token) => Number.parseInt(token, 16));
}

function jsonBytes(value) {
  return Uint8Array.from(Buffer.from(JSON.stringify(value), "utf8"));
}

async function readRows(stream) {
  const reader = stream.getReader();
  const rows = [];
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return rows;
    rows.push(JSON.parse(Buffer.from(value).toString("utf8")));
  }
}

const database = new WasmDatabase(fixture());
for (const [id, name] of [
  [2, "second"],
  [3, "third"],
]) {
  database.apply_operation_json(
    jsonBytes({
      method: "POST",
      path: "/records",
      body: { ID: id, NAME: name, AGE: id, ACTIVE: true },
    }),
  );
}

const query = { projection: { NAME: 1 }, limit: 3 };
const expected = JSON.parse(
  Buffer.from(database.query_json(jsonBytes(query))).toString("utf8"),
);
assert.deepEqual(
  await readRows(createWorkerQueryStream({ database, query, queueSize: 1 })),
  expected,
);

const controller = new AbortController();
const readable = createWorkerQueryStream({
  database,
  query: { projection: { NAME: 1 } },
  signal: controller.signal,
  queueSize: 1,
});
const reader = readable.getReader();
const first = await reader.read();
assert.equal(first.done, false);
controller.abort("client disconnected");
await assert.rejects(
  () => reader.read(),
  (error) =>
    error instanceof WorkerQueryStreamError &&
    error.code === "cancelled" &&
    error.message.includes("client disconnected"),
);

const direct = database.query_stream_json(jsonBytes({}));
direct.cancel();
assert.throws(() => direct.next_json(), /cancelled/);

assert.throws(
  () => createWorkerQueryStream({ database, queueSize: 0 }),
  /positive safe integer/,
);
assert.throws(
  () => createWorkerQueryStream({ database, query: { sort: { ID: 1 } } }),
  /streaming query supports/,
);

let resolvePendingStream;
let pendingStreamCancelled = false;
const asyncDatabase = {
  query_stream_json() {
    return new Promise((resolve) => {
      resolvePendingStream = resolve;
    });
  },
};
const loadingController = new AbortController();
const loadingReader = createWorkerQueryStream({
  database: asyncDatabase,
  signal: loadingController.signal,
}).getReader();
const loadingRead = loadingReader.read();
loadingController.abort("cancel during stream initialization");
await assert.rejects(
  loadingRead,
  (error) =>
    error instanceof WorkerQueryStreamError &&
    error.code === "cancelled" &&
    error.message.includes("cancel during stream initialization"),
);
resolvePendingStream({
  next_json() {
    assert.fail("cancelled stream must not pull a record");
  },
  cancel() {
    pendingStreamCancelled = true;
  },
});
await Promise.resolve();
assert.equal(pendingStreamCancelled, true);

assert.throws(
  () => createWorkerQueryStream({ database, generation: 0 }),
  /unsigned 64-bit BigInt/,
);

const alreadyAborted = new AbortController();
alreadyAborted.abort("already closed");
assert.throws(
  () => createWorkerQueryStream({ database, signal: alreadyAborted.signal }),
  (error) => error.code === "cancelled" && error.message === "already closed",
);

console.log("wasm worker query stream smoke passed");
