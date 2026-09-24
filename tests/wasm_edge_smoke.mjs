import assert from "node:assert/strict";
import path from "node:path";
import { createRequire } from "node:module";

const packageDirectory = path.resolve(process.argv[2] ?? "target/wasm-bindgen");
const require = createRequire(import.meta.url);
const { WasmObjectTable } = require(path.join(packageDirectory, "txbase.js"));

const xbf = Uint8Array.from(
  Buffer.from("VFhCRgEAAAAAAAAAZAAAAGQAAAAAAAAAKwAAAAAAAADx2z6tjwAAAAAAAAAwAAAAAAAAAJs4vMK/AAAAAAAAAFIAAAAAAAAAiF8HnAIAAAAAAAAAAAAAAAAAAAB76dDSAAAAAAQAAAACAElEEQAAAAQATkFNRTAAAAADAEFHRREAAAAGAEFDVElWRQEAAAC/AAAAAAAAACoAAAAAAAAAAAAAAAAAAADpAAAAAAAAACgAAAAAAAAAAQAAAAAAAAARCAAAAAEAAAAAAAAAMAUAAABBbGljZREIAAAAHQAAAAAAAAABAQAAAAERCAAAAAIAAAAAAAAAMAMAAABCb2IRCAAAAAcAAAAAAAAAAQEAAAAA", "base64"),
);

function copyBytes(value) {
  return value === null ? null : Uint8Array.from(value);
}

function equalBytes(left, right) {
  if (left === null || right === null) return left === right;
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
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

host.objects.set("users/snapshots/999.xbf", new Uint8Array([0xff]));
assert.deepEqual(JSON.parse(await table.cleanup_orphans()), ["users/snapshots/999.xbf"]);
assert.deepEqual(JSON.parse(await table.retain_generations(1)), []);
await assert.rejects(() => table.retain_generations(0), /retention count must be positive/);

assert.throws(() => new WasmObjectTable({}, "users"), /host method get/);

console.log("wasm-bindgen async object-table smoke passed");
