import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

import { createR2ObjectStore } from "../src/r2-object-store.mjs";

const packageDirectory = path.resolve(process.argv[2] ?? "target/wasm-bindgen");
const require = createRequire(import.meta.url);
const { WasmObjectTable } = require(path.join(packageDirectory, "txbase.js"));
const xbfHex = await readFile(
  new URL("./fixtures/query-stream-users.xbf.hex", import.meta.url),
  "utf8",
);
const xbf = Uint8Array.from(Buffer.from(xbfHex.replaceAll(/\s/g, ""), "hex"));
const bucket = new FakeR2Bucket(1);

assert.throws(() => createR2ObjectStore(null), /bucket binding must be an object/);

const store = createR2ObjectStore(bucket);
const table = new WasmObjectTable(store, "users");
assert.equal(await table.manifest_json(), null);
assert.equal(await table.read_xbf(), null);
assert.deepEqual(JSON.parse(await table.commit_xbf(xbf)), {
  status: "committed",
  generation: 0,
});
assert.deepEqual([...await table.read_xbf()], [...xbf]);
assert.deepEqual(await store.list("users/"), [
  "users/manifest.json",
  "users/snapshots/0.xbf",
]);
assert.equal(await store.get("users/missing.xbf"), null);

await assert.rejects(
  () => store.putIfAbsent("users/manifest.json", xbf),
  (error) => error.code === "conflict",
);
await assert.rejects(
  () => store.compareAndSwap("users/manifest.json", Uint8Array.of(1), xbf),
  (error) => error.code === "conflict",
);

await store.putIfAbsent("race/key", Uint8Array.of(0));
const competingWrites = await Promise.allSettled([
  store.compareAndSwap("race/key", Uint8Array.of(0), Uint8Array.of(1)),
  store.compareAndSwap("race/key", Uint8Array.of(0), Uint8Array.of(2)),
]);
assert.equal(competingWrites.filter((result) => result.status === "fulfilled").length, 1);
assert.equal(
  competingWrites.filter((result) => result.status === "rejected" && result.reason.code === "conflict")
    .length,
  1,
);

await store.delete("race/key");
await store.delete("race/key");
assert.equal(await store.get("race/key"), null);
await assert.rejects(
  () => store.get("bad\0key"),
  (error) => error.code === "invalid",
);

bucket.repeatCursor = true;
await assert.rejects(
  () => store.list("users/"),
  (error) => error.code === "invalid" && error.message.includes("repeated"),
);
bucket.repeatCursor = false;

const unavailableBucket = Object.create(bucket);
unavailableBucket.get = async () => {
  throw new Error("binding unavailable");
};
const unavailableStore = createR2ObjectStore(unavailableBucket);
await assert.rejects(
  () => unavailableStore.get("users/manifest.json"),
  (error) => error.code === "unavailable",
);

console.log("wasm R2 object-store smoke passed");

class FakeR2Bucket {
  constructor(pageSize) {
    this.objects = new Map();
    this.pageSize = pageSize;
    this.repeatCursor = false;
  }

  async get(key) {
    const entry = this.objects.get(key);
    if (entry === undefined) return null;
    const bytes = entry.bytes.slice();
    return {
      ...entry,
      async arrayBuffer() {
        return bytes.buffer;
      },
    };
  }

  async put(key, value, { onlyIf }) {
    const current = this.objects.get(key);
    if (onlyIf.get("If-None-Match") === "*" && current !== undefined) return null;
    if (
      onlyIf.has("If-Match") &&
      (current === undefined || current.httpEtag !== onlyIf.get("If-Match"))
    ) {
      return null;
    }
    const bytes = Uint8Array.from(value);
    const etag = createHash("sha256").update(bytes).digest("hex");
    const result = { bytes, etag, httpEtag: `"${etag}"` };
    this.objects.set(key, result);
    return { etag, httpEtag: result.httpEtag };
  }

  async delete(key) {
    this.objects.delete(key);
  }

  async list({ prefix, cursor, limit }) {
    const keys = [...this.objects.keys()].filter((key) => key.startsWith(prefix)).sort();
    const offset = cursor === undefined ? 0 : cursor === "same" ? 0 : Number(cursor);
    const page = keys.slice(offset, offset + Math.min(limit, this.pageSize));
    const nextOffset = offset + page.length;
    const truncated = nextOffset < keys.length;
    return {
      objects: page.map((key) => ({ key })),
      truncated,
      ...(truncated ? { cursor: this.repeatCursor ? "same" : String(nextOffset) } : {}),
    };
  }
}
