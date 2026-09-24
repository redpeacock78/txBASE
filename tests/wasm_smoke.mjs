import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

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

function decodeJson(bytes) {
  return JSON.parse(Buffer.from(bytes).toString("utf8"));
}

function assertError(callback, fragment) {
  assert.throws(callback, (error) => String(error).includes(fragment));
}

assert.equal(WasmDatabase.abi_version(), 1);

const initial = fixture();
const database = new WasmDatabase(initial);
assert.deepEqual([...database.snapshot()], [...initial]);
assert.equal(decodeJson(database.query_json(jsonBytes({}))).length, 1);

database.apply_operation_json(
  jsonBytes({
    method: "PUT",
    path: "/records/1",
    body: { ID: 1, NAME: "wasm-put", AGE: 31, ACTIVE: true },
  }),
);
let rows = decodeJson(database.query_json(jsonBytes({})));
assert.equal(rows[0].NAME, "wasm-put");

database.apply_operation_json(
  jsonBytes({
    method: "POST",
    path: "/records",
    body: { ID: 2, NAME: "wasm-post", AGE: 42, ACTIVE: true },
  }),
);
database.apply_operation_json(
  jsonBytes({
    method: "DELETE",
    path: "/records/2",
  }),
);
rows = decodeJson(database.query_json(jsonBytes({})));
assert.deepEqual(rows.map((row) => row.NAME), ["wasm-put"]);

database.apply_operations_json(
  jsonBytes({
    operations: [
      {
        method: "PATCH",
        path: "/records/1",
        body: { NAME: "wasm-batch" },
      },
      {
        method: "POST",
        path: "/records",
        body: { ID: 3, NAME: "wasm-post-batch", AGE: 42, ACTIVE: true },
      },
    ],
  }),
);
rows = decodeJson(database.query_json(jsonBytes({ sort: { ID: 1 } })));
assert.deepEqual(
  rows.map((row) => row.NAME),
  ["wasm-batch", "wasm-post-batch"],
);

const beforeFailedBatch = [...database.snapshot()];
assertError(
  () =>
    database.apply_operations_json(
      jsonBytes({
        operations: [
          {
            method: "PATCH",
            path: "/records/1",
            body: { NAME: "must-not-publish" },
          },
          { method: "DELETE", path: "/records/0" },
        ],
      }),
    ),
  "operation record id must be positive",
);
assert.deepEqual([...database.snapshot()], beforeFailedBatch);

const restored = new WasmDatabase(database.snapshot());
rows = decodeJson(restored.query_json(jsonBytes({ sort: { ID: 1 } })));
assert.deepEqual(
  rows.map((row) => row.NAME),
  ["wasm-batch", "wasm-post-batch"],
);

console.log("wasm-bindgen Node smoke passed");
