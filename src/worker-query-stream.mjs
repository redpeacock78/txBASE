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
  textEncoder = new TextEncoder(),
} = {}) {
  if (!database || typeof database.query_stream_json !== "function") {
    throw new WorkerQueryStreamError(
      "invalid",
      "database must expose query_stream_json",
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
  try {
    coreStream = database.query_stream_json(normalizeQueryBody(query, textEncoder));
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
    try {
      coreStream.cancel();
    } catch {
      // The stream is already being cancelled; the consumer-visible error wins.
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
      pull(streamController) {
        if (closed || cancelled) return;
        try {
          const record = coreStream.next_json();
          if (record === null || record === undefined) {
            closed = true;
            cleanup();
            streamController.close();
            return;
          }
          streamController.enqueue(textEncoder.encode(`${record}\n`));
        } catch (error) {
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
