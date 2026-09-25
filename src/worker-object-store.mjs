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
    !isAbortSignal(signal)
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

  const request = async (url, init = {}, operationSignal, consumeResponse) => {
    const callerSignals = [
      ...new Set([signal, operationSignal].filter((item) => item !== undefined)),
    ];
    for (const callerSignal of callerSignals) {
      if (callerSignal.aborted) throw cancelledError(callerSignal.reason);
    }
    const controller = new AbortController();
    let timedOut = false;
    let callerAborted = false;
    let callerReason;
    const abortListeners = callerSignals.map((callerSignal) => {
      const listener = () => {
        callerAborted = true;
        callerReason = callerSignal.reason;
        controller.abort(callerReason);
      };
      callerSignal.addEventListener("abort", listener, { once: true });
      return [callerSignal, listener];
    });
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
      const response = await fetchImpl(url, {
        ...init,
        cache,
        headers: requestHeaders,
        signal: controller.signal,
      });
      return await consumeResponse(response);
    } catch (error) {
      if (timedOut) {
        throw new WorkerObjectStoreError(
          "unavailable",
          `object-store request timed out after ${timeoutMs} ms`,
        );
      }
      if (callerAborted) {
        throw cancelledError(callerReason);
      }
      if (error instanceof WorkerObjectStoreError) throw error;
      throw new WorkerObjectStoreError(
        "unavailable",
        `object-store request failed: ${errorMessage(error)}`,
      );
    } finally {
      if (timer !== undefined) clearTimeout(timer);
      for (const [callerSignal, listener] of abortListeners) {
        callerSignal.removeEventListener("abort", listener);
      }
    }
  };

  const validateOperationSignal = (operationSignal) => {
    if (operationSignal !== undefined && !isAbortSignal(operationSignal)) {
      throw new WorkerObjectStoreError(
        "invalid",
        "worker object-store operation signal must be an AbortSignal",
      );
    }
  };

  const get = async (key, operationSignal) => {
    validateOperationSignal(operationSignal);
    return request(keyUrl(root, key), {}, operationSignal, async (response) => {
      if (response.status === 404) return null;
      await requireStatus(response, "get", [200]);
      return new Uint8Array(await response.arrayBuffer());
    });
  };

  const putIfAbsent = async (key, bytes, operationSignal) => {
    validateOperationSignal(operationSignal);
    await request(keyUrl(root, key), {
      method: "PUT",
      headers: { "If-None-Match": "*" },
      body: bytes,
    }, operationSignal, (response) => requireStatus(response, "put-if-absent", [200, 201, 204]));
  };

  const compareAndSwap = async (key, expected, replacement, operationSignal) => {
    validateOperationSignal(operationSignal);
    const conditionalHeaders = expected === null ? { "If-None-Match": "*" } : {
      "If-Match": await strongEtag(expected, cryptoImpl),
    };
    await request(keyUrl(root, key), {
      method: "PUT",
      headers: conditionalHeaders,
      body: replacement,
    }, operationSignal, (response) => requireStatus(response, "compare-and-swap", [200, 201, 204]));
  };

  const remove = async (key, operationSignal) => {
    validateOperationSignal(operationSignal);
    await request(keyUrl(root, key), {
      method: "DELETE",
    }, operationSignal, (response) => requireStatus(response, "delete", [200, 204, 404]));
  };

  const list = async (prefix, operationSignal) => {
    validateOperationSignal(operationSignal);
    validatePrefix(prefix);
    const url = new URL(root);
    url.searchParams.set("prefix", prefix);
    return request(url, {}, operationSignal, async (response) => {
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
    });
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

function validatePrefix(prefix) {
  if (
    typeof prefix !== "string" ||
    prefix.length === 0 ||
    prefix.includes("\0") ||
    prefix.split("/").some(
      (component, index, components) =>
        component === "." ||
        component === ".." ||
        (component === "" && index !== components.length - 1),
    )
  ) {
    throw new WorkerObjectStoreError(
      "invalid",
      "object-store list prefixes must contain ordinary path components",
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

function isAbortSignal(signal) {
  return (
    signal !== null &&
    typeof signal === "object" &&
    typeof signal.aborted === "boolean" &&
    typeof signal.addEventListener === "function" &&
    typeof signal.removeEventListener === "function"
  );
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
