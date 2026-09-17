/**
 * Fake echo harness used by the external-adapter conformance fixtures.
 *
 * Speaks one JSON object per line on stdio. Two line kinds:
 * - `{ kind: "res", reqId, ok, result }` — responses to package requests.
 * - `{ kind: "ev", reqId?, threadId, turnId?, type, payload }` — protocol
 *   events the package translates into canonical runtime events.
 *
 * Requests: `{ reqId, type, threadId, ... }` where type is one of
 * `start`, `send` (`respondAt: "started" | "completed"`, `hold`),
 * `interrupt`, `readThread`, `rollback` (`numTurns`), `stop`.
 * A `hold` turn stays open until an `interrupt` arrives.
 */
let seq = 0;
const threads = new Map();

const activeTurn = (threadId) => {
  const thread = threads.get(threadId);
  return thread?.turns.find((turn) => turn.open);
};

const respond = (reqId, ok, result) => {
  process.stdout.write(`${JSON.stringify({ kind: "res", reqId, ok, result })}
`);
};

const emit = (event) => {
  process.stdout.write(`${JSON.stringify({ kind: "ev", ...event })}
`);
};

process.stdin.setEncoding("utf8");
let buffered = "";
process.stdin.on("data", (chunk) => {
  buffered += chunk;
  for (;;) {
    const index = buffered.indexOf("\n");
    if (index < 0) break;
    const line = buffered.slice(0, index);
    buffered = buffered.slice(index + 1);
    if (!line.trim()) continue;
    const request = JSON.parse(line);
    seq += 1;
    const threadId = request.threadId;

    switch (request.type) {
      case "start": {
        if (!threads.has(threadId)) threads.set(threadId, { turns: [] });
        emit({ reqId: request.reqId, threadId, type: "session.started", payload: {} });
        respond(request.reqId, true, { started: true });
        break;
      }
      case "send": {
        if (!threads.has(threadId)) threads.set(threadId, { turns: [] });
        const turnId = `turn-${seq}`;
        const turn = {
          id: turnId,
          open: Boolean(request.hold),
          input: request.input ?? "",
          interrupted: false,
        };
        threads.get(threadId).turns.push(turn);
        emit({ reqId: request.reqId, threadId, turnId, type: "turn.started", payload: {} });
        if (request.respondAt === "started") {
          respond(request.reqId, true, { turnId });
          if (!request.hold) {
            emit({
              reqId: request.reqId,
              threadId,
              turnId,
              type: "content.delta",
              payload: {
                streamKind: "assistant_text",
                delta: String(request.input ?? "")
                  .split("")
                  .toReversed()
                  .join(""),
              },
            });
            emit({
              reqId: request.reqId,
              threadId,
              turnId,
              type: "turn.completed",
              payload: { state: "completed" },
            });
            turn.open = false;
          }
        } else {
          emit({
            reqId: request.reqId,
            threadId,
            turnId,
            type: "content.delta",
            payload: {
              streamKind: "assistant_text",
              delta: String(request.input ?? "")
                .split("")
                .toReversed()
                .join(""),
            },
          });
          emit({
            reqId: request.reqId,
            threadId,
            turnId,
            type: "turn.completed",
            payload: { state: "completed" },
          });
          turn.open = false;
          respond(request.reqId, true, { turnId });
        }
        break;
      }
      case "interrupt": {
        const turn = activeTurn(threadId);
        if (turn) {
          turn.open = false;
          turn.interrupted = true;
          emit({
            reqId: request.reqId,
            threadId,
            turnId: turn.id,
            type: "turn.aborted",
            payload: { reason: "interrupted by echo harness" },
          });
          respond(request.reqId, true, { interrupted: true });
        } else {
          respond(request.reqId, true, { interrupted: false });
        }
        break;
      }
      case "readThread": {
        const thread = threads.get(threadId);
        respond(request.reqId, true, {
          threadId,
          turns: (thread?.turns ?? []).map((turn) => ({
            id: turn.id,
            items: [{ text: turn.input, interrupted: turn.interrupted }],
          })),
        });
        break;
      }
      case "rollback": {
        const thread = threads.get(threadId);
        const turns = thread?.turns ?? [];
        turns.splice(Math.max(0, turns.length - (request.numTurns ?? 0)));
        respond(request.reqId, true, {
          threadId,
          turns: turns.map((turn) => ({
            id: turn.id,
            items: [{ text: turn.input, interrupted: turn.interrupted }],
          })),
        });
        break;
      }
      case "stop": {
        emit({
          reqId: request.reqId,
          threadId,
          type: "session.exited",
          payload: { exitKind: "graceful" },
        });
        respond(request.reqId, true, { stopped: true });
        process.exit(0);
        break;
      }
      default:
        respond(request.reqId, false, { error: `unknown request ${request.type}` });
    }
  }
});
process.stdin.on("end", () => process.exit(0));
