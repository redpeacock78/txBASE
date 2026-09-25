import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { createWorkerQueryStream } from "../src/worker-query-stream.mjs";

const packageDirectory = path.resolve(process.argv[2] ?? "target/wasm-bindgen");
const require = createRequire(import.meta.url);
const { WasmObjectTable } = require(path.join(packageDirectory, "txbase.js"));

// Keep the schema DBF-exportable so this fixture exercises the shared query path.
const xbfHex = fs
  .readFileSync(new URL("./fixtures/query-stream-users.xbf.hex", import.meta.url), "utf8")
  .replace(/\s+/g, "");
const xbf = Uint8Array.from(
  xbfHex.match(/.{2}/g),
  (byte) => Number.parseInt(byte, 16),
);

function copyBytes(value) {
  return value === null ? null : Uint8Array.from(value);
}

function equalBytes(left, right) {
  if (left === null || right === null) return left === right;
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
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

class HostObjectStore {
  constructor() {
    this.objects = new Map();
    this.failDeleteOnce = false;
  }

  async get(key) {
    return copyBytes(this.objects.get(key) ?? null);
  }

  async putIfAbsent(key, bytes) {
    if (this.objects.has(key)) {
      throw { code: "conflict", message: `object already exists: ${key}` };
    }
    this.objects.set(key, copyBytes(bytes));
  }

  async compareAndSwap(key, expected, replacement) {
    const current = copyBytes(this.objects.get(key) ?? null);
    if (!equalBytes(current, expected)) {
      throw { code: "conflict", message: `compare-and-swap conflict: ${key}` };
    }
    this.objects.set(key, copyBytes(replacement));
  }

  async delete(key) {
    if (this.failDeleteOnce) {
      this.failDeleteOnce = false;
      throw { code: "unavailable", message: "temporary delete failure" };
    }
    this.objects.delete(key);
  }

  async list(prefix) {
    return [...this.objects.keys()].filter((key) => key.startsWith(prefix)).sort();
  }
}

const host = new HostObjectStore();
const table = new WasmObjectTable(host, "users");
assert.equal(table.manifest_key(), "users/manifest.json");
assert.equal(await table.manifest_json(), null);
assert.equal(await table.read_xbf(), null);

host.failDeleteOnce = true;
await assert.rejects(() => table.commit_xbf(xbf), /temporary delete failure/);
assert.equal(JSON.parse(await table.manifest_json()).generation, 0);
assert.equal(await table.recover(), 1);

const committed = JSON.parse(await table.commit_xbf(xbf));
assert.deepEqual(committed, { status: "already_committed", generation: 0 });
assert.deepEqual([...await table.read_xbf()], [...xbf]);
assert.deepEqual([...await table.read_xbf_at(0n)], [...xbf]);
assert.equal(await table.read_xbf_at(1n), null);

const query = { projection: { NAME: 1 } };
const currentRows = await readRows(
  createWorkerQueryStream({ database: table, query, queueSize: 1 }),
);
assert.deepEqual(currentRows, [{ NAME: "Alice" }, { NAME: "Bob" }]);
const historicalRows = await readRows(
  createWorkerQueryStream({ database: table, generation: 0n, query, queueSize: 1 }),
);
assert.deepEqual(historicalRows, currentRows);
const cancelledStream = await table.query_stream_json(jsonBytes(query));
cancelledStream.cancel();
assert.throws(() => cancelledStream.next_json(), /cancelled/);
await assert.rejects(
  () => table.query_stream_json_at(1n, jsonBytes(query)),
  /no retained snapshot at generation 1/,
);

host.objects.set("users/snapshots/999.xbf", new Uint8Array([0xff]));
assert.deepEqual(JSON.parse(await table.cleanup_orphans()), ["users/snapshots/999.xbf"]);
assert.deepEqual(JSON.parse(await table.retain_generations(1)), []);
await assert.rejects(() => table.retain_generations(0), /retention count must be positive/);

assert.throws(() => new WasmObjectTable({}, "users"), /host method get/);

console.log("wasm-bindgen async object-table smoke passed");
