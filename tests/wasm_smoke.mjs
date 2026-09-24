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

assert.equal(WasmDatabase.abi_version(), 1);

const database = new WasmDatabase(fixture());
assert.equal(decodeJson(database.query_json(jsonBytes({}))).length, 1);

database.apply_operation_json(
  jsonBytes({
    method: "PATCH",
    path: "/records/1",
    body: { NAME: "wasm-node" },
  }),
);
let rows = decodeJson(database.query_json(jsonBytes({})));
assert.equal(rows[0].NAME, "wasm-node");

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
        body: { ID: 3, NAME: "second", AGE: 42, ACTIVE: true },
      },
    ],
  }),
);
rows = decodeJson(database.query_json(jsonBytes({ sort: { ID: 1 } })));
assert.deepEqual(
  rows.map((row) => row.NAME),
  ["wasm-batch", "second"],
);

console.log("wasm-bindgen Node smoke passed");
