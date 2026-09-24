const DEFAULT_TIMEOUT_MS = 30_000;
const DEFAULT_CACHE_MODE = "no-store";

export class WorkerObjectStoreError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "WorkerObjectStoreError";
    this.code = code;
  }
}

/**
 * Create the five-operation host object expected by WasmObjectTable.
 *
 * The service exposes objects below `baseUrl` and accepts a `prefix` query
 * parameter on the collection URL for list operations. Conditional writes
 * use standard HTTP preconditions: If-None-Match for put-if-absent and a
 * SHA-256 strong ETag in If-Match for compare-and-swap.
 */
export function createWorkerObjectStore({
  baseUrl,
  fetchImpl = globalThis.fetch,
  headers = {},
  signal,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  cache = DEFAULT_CACHE_MODE,
  cryptoImpl = globalThis.crypto,
} = {}) {
  const root = validateBaseUrl(baseUrl);
  if (typeof fetchImpl !== "function") {
    throw new WorkerObjectStoreError(
      "invalid",
      "worker object-store adapter requires a fetch implementation",
    );
  }
  if (!Number.isInteger(timeoutMs) || timeoutMs < 0) {
    throw new WorkerObjectStoreError(
      "invalid",
      "worker object-store timeout must be a non-negative integer",
    );
  }
  if (
    signal !== undefined &&
    (typeof signal !== "object" ||
      signal === null ||
      typeof signal.aborted !== "boolean" ||
      typeof signal.addEventListener !== "function" ||
      typeof signal.removeEventListener !== "function")
  ) {
    throw new WorkerObjectStoreError(
      "invalid",
      "worker object-store signal must be an AbortSignal",
    );
  }
  let defaultHeaders;
  try {
    defaultHeaders = new Headers(headers);
  } catch (error) {
    throw new WorkerObjectStoreError(
      "invalid",
      `worker object-store headers are invalid: ${errorMessage(error)}`,
    );
  }

  const request = async (url, init = {}) => {
    if (signal?.aborted) {
      throw cancelledError(signal.reason);
    }
    const controller = new AbortController();
    let timedOut = false;
    let callerAborted = false;
    const abortFromCaller = () => {
      callerAborted = true;
      controller.abort(signal.reason);
    };
    signal?.addEventListener("abort", abortFromCaller, { once: true });
    const timer =
      timeoutMs > 0
        ? setTimeout(() => {
            timedOut = true;
            controller.abort();
          }, timeoutMs)
        : undefined;
    try {
      const requestHeaders = new Headers(defaultHeaders);
      for (const [key, value] of new Headers(init.headers ?? {})) {
        requestHeaders.set(key, value);
      }
      return await fetchImpl(url, {
        ...init,
        cache,
        headers: requestHeaders,
        signal: controller.signal,
      });
    } catch (error) {
      if (timedOut) {
        throw new WorkerObjectStoreError(
          "unavailable",
          `object-store request timed out after ${timeoutMs} ms`,
        );
      }
      if (callerAborted || controller.signal.aborted) {
        throw cancelledError(signal?.reason);
      }
      throw new WorkerObjectStoreError(
        "unavailable",
        `object-store request failed: ${errorMessage(error)}`,
      );
    } finally {
      if (timer !== undefined) clearTimeout(timer);
      signal?.removeEventListener("abort", abortFromCaller);
    }
  };

  const get = async (key) => {
    const response = await request(keyUrl(root, key));
    if (response.status === 404) return null;
    await requireStatus(response, "get", [200]);
    return new Uint8Array(await response.arrayBuffer());
  };

  const putIfAbsent = async (key, bytes) => {
    const response = await request(keyUrl(root, key), {
      method: "PUT",
      headers: { "If-None-Match": "*" },
      body: bytes,
    });
    await requireStatus(response, "put-if-absent", [200, 201, 204]);
  };

  const compareAndSwap = async (key, expected, replacement) => {
    const conditionalHeaders = expected === null ? { "If-None-Match": "*" } : {
      "If-Match": await strongEtag(expected, cryptoImpl),
    };
    const response = await request(keyUrl(root, key), {
      method: "PUT",
      headers: conditionalHeaders,
      body: replacement,
    });
    await requireStatus(response, "compare-and-swap", [200, 201, 204]);
  };

  const remove = async (key) => {
    const response = await request(keyUrl(root, key), { method: "DELETE" });
    await requireStatus(response, "delete", [200, 204, 404]);
  };

  const list = async (prefix) => {
    validateKey(prefix);
    const url = new URL(root);
    url.searchParams.set("prefix", prefix);
    const response = await request(url);
    await requireStatus(response, "list", [200]);
    let keys;
    try {
      keys = await response.json();
    } catch (error) {
      throw new WorkerObjectStoreError(
        "invalid",
        `list response is not valid JSON: ${errorMessage(error)}`,
      );
    }
    if (!Array.isArray(keys) || keys.some((key) => typeof key !== "string")) {
      throw new WorkerObjectStoreError(
        "invalid",
        "list response must be a JSON array of strings",
      );
    }
    return [...keys].sort();
  };

  return { get, putIfAbsent, compareAndSwap, delete: remove, list };
}

async function requireStatus(response, operation, successStatuses) {
  if (successStatuses.includes(response.status)) return;
  const message = await response.text().catch(() => "");
  const detail = message ? `: ${message.slice(0, 256)}` : "";
  throw new WorkerObjectStoreError(
    statusCode(response.status),
    `object-store ${operation} returned HTTP ${response.status}${detail}`,
  );
}

function statusCode(status) {
  if (status === 404) return "missing";
  if (status === 409 || status === 412) return "conflict";
  if (status >= 400 && status < 500 && status !== 408 && status !== 429) {
    return "invalid";
  }
  return "unavailable";
}

function validateBaseUrl(value) {
  if (typeof value !== "string" || value.length === 0) {
    throw new WorkerObjectStoreError("invalid", "baseUrl must be a non-empty URL");
  }
  let url;
  try {
    url = new URL(value);
  } catch (error) {
    throw new WorkerObjectStoreError(
      "invalid",
      `baseUrl is not a valid URL: ${errorMessage(error)}`,
    );
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new WorkerObjectStoreError(
      "invalid",
      "baseUrl must use http or https",
    );
  }
  url.search = "";
  url.hash = "";
  if (!url.pathname.endsWith("/")) url.pathname += "/";
  return url;
}

function keyUrl(root, key) {
  validateKey(key);
  const url = new URL(root);
  url.pathname += key
    .split("/")
    .map((component) => encodeURIComponent(component))
    .join("/");
  return url;
}

function validateKey(key) {
  if (
    typeof key !== "string" ||
    key.length === 0 ||
    key.includes("\0") ||
    key
      .split("/")
      .some((component) => component === "" || component === "." || component === "..")
  ) {
    throw new WorkerObjectStoreError(
      "invalid",
      "object-store keys must contain ordinary non-empty path components",
    );
  }
}

async function strongEtag(bytes, cryptoImpl) {
  if (!cryptoImpl?.subtle?.digest) {
    throw new WorkerObjectStoreError(
      "invalid",
      "worker object-store adapter requires Web Crypto SHA-256 for compare-and-swap",
    );
  }
  const digest = new Uint8Array(
    await cryptoImpl.subtle.digest("SHA-256", bytes),
  );
  return `"${base64Url(digest)}"`;
}

function base64Url(bytes) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

function cancelledError(reason) {
  return new WorkerObjectStoreError(
    "cancelled",
    `object-store request was cancelled${reason ? `: ${errorMessage(reason)}` : ""}`,
  );
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}
