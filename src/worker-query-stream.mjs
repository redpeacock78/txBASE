const DEFAULT_QUEUE_SIZE = 1;

export class WorkerQueryStreamError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "WorkerQueryStreamError";
    this.code = code;
  }
}

export function createWorkerQueryStream({
  database,
  query = {},
  signal,
  queueSize = DEFAULT_QUEUE_SIZE,
  generation,
  textEncoder = new TextEncoder(),
} = {}) {
  if (
    generation !== undefined &&
    (typeof generation !== "bigint" || generation < 0n || generation > 0xffff_ffff_ffff_ffffn)
  ) {
    throw new WorkerQueryStreamError(
      "invalid",
      "query stream generation must be an unsigned 64-bit BigInt",
    );
  }
  const method = generation === undefined
    ? database?.query_stream_json
    : database?.query_stream_json_at;
  if (typeof method !== "function") {
    throw new WorkerQueryStreamError(
      "invalid",
      generation === undefined
        ? "database must expose query_stream_json"
        : "database must expose query_stream_json_at for a selected generation",
    );
  }
  if (!Number.isSafeInteger(queueSize) || queueSize < 1) {
    throw new WorkerQueryStreamError(
      "invalid",
      "query stream queue size must be a positive safe integer",
    );
  }
  if (!textEncoder || typeof textEncoder.encode !== "function") {
    throw new WorkerQueryStreamError(
      "invalid",
      "query stream textEncoder must expose encode",
    );
  }
  if (signal !== undefined && !isAbortSignal(signal)) {
    throw new WorkerQueryStreamError(
      "invalid",
      "query stream signal must be an AbortSignal",
    );
  }
  if (signal?.aborted) {
    throw cancellationError(signal);
  }

  let coreStream;
  let pendingCoreStream;
  try {
    const body = normalizeQueryBody(query, textEncoder);
    const result = generation === undefined
      ? method.call(database, body)
      : method.call(database, generation, body);
    if (result && typeof result.then === "function") {
      pendingCoreStream = Promise.resolve(result);
    } else {
      coreStream = result;
    }
  } catch (error) {
    throw mapStreamError(error);
  }

  let controller;
  let closed = false;
  let cancelled = false;

  const cleanup = () => {
    signal?.removeEventListener("abort", abort);
  };

  const cancelCore = () => {
    if (cancelled) return;
    cancelled = true;
    if (coreStream) {
      cancelStream(coreStream);
    } else {
      pendingCoreStream?.then(cancelStream, () => {});
    }
  };

  const abort = () => {
    if (closed) return;
    const error = cancellationError(signal);
    cancelCore();
    closed = true;
    cleanup();
    controller?.error(error);
  };

  const readable = new ReadableStream(
    {
      start(streamController) {
        controller = streamController;
        signal?.addEventListener("abort", abort, { once: true });
        if (signal?.aborted) abort();
      },
      async pull(streamController) {
        if (closed || cancelled) return;
        try {
          const stream = coreStream ?? await pendingCoreStream;
          if (closed || cancelled) return;
          if (
            !stream ||
            typeof stream.next_json !== "function" ||
            typeof stream.cancel !== "function"
          ) {
            throw new WorkerQueryStreamError(
              "invalid",
              "query stream method must return an object with next_json and cancel",
            );
          }
          coreStream = stream;
          const record = await stream.next_json();
          if (closed || cancelled) return;
          if (record === null || record === undefined) {
            closed = true;
            cleanup();
            streamController.close();
            return;
          }
          streamController.enqueue(textEncoder.encode(`${record}\n`));
        } catch (error) {
          if (closed || cancelled) return;
          closed = true;
          cleanup();
          streamController.error(mapStreamError(error));
        }
      },
      cancel() {
        if (closed) return;
        closed = true;
        cancelCore();
        cleanup();
      },
    },
    { highWaterMark: queueSize, size: () => 1 },
  );

  return readable;
}

function cancelStream(stream) {
  try {
    stream?.cancel();
  } catch {
    // The consumer-visible error wins if cancellation itself fails.
  }
}

function normalizeQueryBody(query, textEncoder) {
  if (query instanceof Uint8Array) return query;
  if (query instanceof ArrayBuffer) return new Uint8Array(query);
  if (ArrayBuffer.isView(query)) {
    return new Uint8Array(query.buffer, query.byteOffset, query.byteLength);
  }
  let serialized;
  try {
    serialized = JSON.stringify(query);
  } catch (error) {
    throw new WorkerQueryStreamError(
      "invalid",
      `query could not be serialized: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
  if (serialized === undefined) {
    throw new WorkerQueryStreamError("invalid", "query must be JSON-serializable");
  }
  return textEncoder.encode(serialized);
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

function cancellationError(signal) {
  const reason = signal?.reason;
  const message =
    reason instanceof Error
      ? reason.message
      : reason === undefined
        ? "query stream cancelled"
        : String(reason);
  return new WorkerQueryStreamError("cancelled", message);
}

function mapStreamError(error) {
  if (error instanceof WorkerQueryStreamError) return error;
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("cancelled")) {
    return new WorkerQueryStreamError("cancelled", message);
  }
  if (message.includes("invalid query") || message.includes("query stream")) {
    return new WorkerQueryStreamError("invalid", message);
  }
  return new WorkerQueryStreamError("unavailable", message);
}
