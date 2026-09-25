const LIST_PAGE_SIZE = 1_000;

export class R2ObjectStoreError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "R2ObjectStoreError";
    this.code = code;
  }
}

export function createR2ObjectStore(bucket) {
  if (bucket === null || typeof bucket !== "object") {
    throw new R2ObjectStoreError("invalid", "R2 bucket binding must be an object");
  }
  for (const method of ["get", "put", "delete", "list"]) {
    if (typeof bucket[method] !== "function") {
      throw new R2ObjectStoreError(
        "invalid",
        `R2 bucket binding method ${method} must be a function`,
      );
    }
  }
  const HeadersImpl = globalThis.Headers;
  if (typeof HeadersImpl !== "function") {
    throw new R2ObjectStoreError("invalid", "R2 adapter requires the Headers API");
  }

  const readObject = async (key) => {
    validateKey(key);
    const object = await invoke("get", () => bucket.get(key));
    if (object === null) return null;
    if (!object || typeof object.arrayBuffer !== "function") {
      throw invalid("R2 get must resolve to an object with arrayBuffer() or null");
    }
    const body = await invoke("read", () => object.arrayBuffer());
    if (!(body instanceof ArrayBuffer)) {
      throw invalid("R2 object arrayBuffer() must resolve to an ArrayBuffer");
    }
    return { object, bytes: new Uint8Array(body) };
  };

  const put = async (key, bytes, condition, operation) => {
    validateKey(key);
    validateBytes(bytes, "replacement");
    const result = await invoke(operation, () =>
      bucket.put(key, bytes, { onlyIf: new HeadersImpl(condition) }),
    );
    if (result === null) {
      throw conflict(`R2 ${operation} condition did not match for ${key}`);
    }
    if (!result || typeof result !== "object") {
      throw invalid(`R2 ${operation} must resolve to an object or null`);
    }
  };

  return {
    async get(key) {
      return (await readObject(key))?.bytes ?? null;
    },

    async putIfAbsent(key, bytes) {
      await put(key, bytes, { "If-None-Match": "*" }, "put-if-absent");
    },

    async compareAndSwap(key, expected, replacement) {
      validateKey(key);
      validateBytes(replacement, "replacement");
      if (expected === null) {
        await put(key, replacement, { "If-None-Match": "*" }, "compare-and-swap");
        return;
      }
      validateBytes(expected, "expected value");
      const current = await readObject(key);
      if (current === null || !equalBytes(current.bytes, expected)) {
        throw conflict(`R2 compare-and-swap value did not match for ${key}`);
      }
      if (typeof current.object.httpEtag !== "string" || current.object.httpEtag.length === 0) {
        throw invalid("R2 object must provide a non-empty httpEtag for compare-and-swap");
      }
      await put(
        key,
        replacement,
        { "If-Match": current.object.httpEtag },
        "compare-and-swap",
      );
    },

    async delete(key) {
      validateKey(key);
      await invoke("delete", () => bucket.delete(key));
    },

    async list(prefix) {
      validatePrefix(prefix);
      const keys = [];
      const cursors = new Set();
      let cursor;
      while (true) {
        const options = { prefix, limit: LIST_PAGE_SIZE };
        if (cursor !== undefined) options.cursor = cursor;
        const page = await invoke("list", () => bucket.list(options));
        if (
          !page ||
          !Array.isArray(page.objects) ||
          typeof page.truncated !== "boolean"
        ) {
          throw invalid("R2 list must resolve to an object page with objects and truncated");
        }
        for (const object of page.objects) {
          if (typeof object?.key !== "string" || !object.key.startsWith(prefix)) {
            throw invalid("R2 list returned an object outside the requested prefix");
          }
          keys.push(object.key);
        }
        if (!page.truncated) return keys.sort();
        if (typeof page.cursor !== "string" || cursors.has(page.cursor)) {
          throw invalid("R2 list returned a missing or repeated pagination cursor");
        }
        cursor = page.cursor;
        cursors.add(cursor);
      }
    },
  };
}

async function invoke(operation, callback) {
  try {
    return await callback();
  } catch (error) {
    if (error instanceof R2ObjectStoreError) throw error;
    throw new R2ObjectStoreError(
      "unavailable",
      `R2 ${operation} failed: ${errorMessage(error)}`,
    );
  }
}

function validateKey(key) {
  if (typeof key !== "string" || key.length === 0 || key.includes("\0")) {
    throw invalid("object key must be non-empty and must not contain NUL");
  }
}

function validatePrefix(prefix) {
  if (typeof prefix !== "string" || prefix.length === 0 || prefix.includes("\0")) {
    throw invalid("object-store list prefix must be non-empty and must not contain NUL");
  }
}

function validateBytes(bytes, label) {
  if (!(bytes instanceof Uint8Array)) {
    throw invalid(`object-store ${label} must be a Uint8Array`);
  }
}

function equalBytes(left, right) {
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function conflict(message) {
  return new R2ObjectStoreError("conflict", message);
}

function invalid(message) {
  return new R2ObjectStoreError("invalid", message);
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}
