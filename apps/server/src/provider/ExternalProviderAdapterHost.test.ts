import * as NodeServices from "@effect/platform-node/NodeServices";
import { assert, describe, it } from "@effect/vitest";
import {
  ProviderAdapterPackageId,
  ProviderInstanceId,
  ThreadId,
  type ChatAttachment,
} from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Layer from "effect/Layer";
import * as Path from "effect/Path";

import { resolveAttachmentPath } from "../attachmentStore.ts";
import { ServerConfig, layerTest } from "../config.ts";
import { ProviderAdapterHostResourceError } from "@t3tools/provider-adapter";

import { makeExternalProviderAdapterHostV2 } from "./ExternalProviderAdapterHost.ts";

const PACKAGE_ID = ProviderAdapterPackageId.make("test-adapter");
const INSTANCE_ID = ProviderInstanceId.make("test-instance");
const THREAD_ID = ThreadId.make("thread-123");

const runHostTest = <A, E>(
  effect: Effect.Effect<
    A,
    E,
    | FileSystem.FileSystem
    | Path.Path
    | ServerConfig
    | import("effect/Scope").Scope
    | import("effect/unstable/process/ChildProcessSpawner").ChildProcessSpawner
  >,
) =>
  effect.pipe(
    Effect.scoped,
    Effect.provide(
      Layer.merge(
        NodeServices.layer,
        layerTest(process.cwd(), { prefix: "t3-provider-host-v2-test-" }).pipe(
          Layer.provide(NodeServices.layer),
        ),
      ),
    ),
  );

const makeHost = (storageKey?: string) =>
  makeExternalProviderAdapterHostV2({
    packageId: PACKAGE_ID,
    instanceId: INSTANCE_ID,
    ...(storageKey === undefined ? {} : { storageKey }),
  });

const expectResourceFailure = Effect.fn(function* <A, R>(
  effect: Effect.Effect<A, ProviderAdapterHostResourceError, R>,
) {
  const result = yield* effect.pipe(Effect.result);
  assert.equal(result._tag, "Failure");
  if (result._tag === "Success") {
    return yield* Effect.die("Expected resource operation to fail");
  }
  assert.equal(
    (result.failure as { readonly _tag?: string })._tag,
    "ProviderAdapterHostResourceError",
  );
});

describe("ExternalProviderAdapterHostV2", () => {
  it.effect("uses package/instance storage by default and preserves a legacy storage key", () =>
    runHostTest(
      Effect.gen(function* () {
        const config = yield* ServerConfig;
        const path = yield* Path.Path;
        const defaultHost = yield* makeHost();
        const defaultStorage = yield* defaultHost.host.storage.prepareSession(THREAD_ID);
        assert.equal(
          defaultStorage.sharedDirectory,
          path.resolve(config.baseDir, "provider-adapters", PACKAGE_ID, INSTANCE_ID),
        );
        assert.equal(
          defaultStorage.sessionDirectory,
          path.join(defaultStorage.sharedDirectory, "sessions", THREAD_ID),
        );

        const legacyHost = yield* makeHost("prime-agent");
        const legacyStorage = yield* legacyHost.host.storage.prepareSession(THREAD_ID);
        assert.equal(legacyStorage.sharedDirectory, path.resolve(config.baseDir, "prime-agent"));
        assert.equal(
          legacyStorage.sessionDirectory,
          path.join(config.baseDir, "prime-agent", "sessions", THREAD_ID),
        );
      }),
    ),
  );

  it.effect("rejects invalid legacy storage keys and thread traversal", () =>
    runHostTest(
      Effect.gen(function* () {
        yield* expectResourceFailure(makeHost("../outside"));
        const broker = yield* makeHost();
        yield* expectResourceFailure(
          broker.host.storage.prepareSession(ThreadId.make("../outside")),
        );
      }),
    ),
  );

  it.effect("resolves default/requested cwd and rejects missing or non-directory paths", () =>
    runHostTest(
      Effect.gen(function* () {
        const config = yield* ServerConfig;
        const fs = yield* FileSystem.FileSystem;
        const path = yield* Path.Path;
        const broker = yield* makeHost();
        assert.equal(yield* broker.host.workspaces.resolveCwd(), yield* fs.realPath(config.cwd));

        const requested = path.join(config.baseDir, "workspace");
        yield* fs.makeDirectory(requested);
        assert.equal(
          yield* broker.host.workspaces.resolveCwd(requested),
          yield* fs.realPath(requested),
        );

        const file = path.join(config.baseDir, "not-a-directory");
        yield* fs.writeFileString(file, "file");
        yield* expectResourceFailure(broker.host.workspaces.resolveCwd(file));
        yield* expectResourceFailure(
          broker.host.workspaces.resolveCwd(path.join(config.baseDir, "missing")),
        );
      }),
    ),
  );

  it.effect("validates existing and future session files within the thread directory", () =>
    runHostTest(
      Effect.gen(function* () {
        const fs = yield* FileSystem.FileSystem;
        const path = yield* Path.Path;
        const config = yield* ServerConfig;
        const broker = yield* makeHost();
        const storage = yield* broker.host.storage.prepareSession(THREAD_ID);
        const existing = path.join(storage.sessionDirectory, "state.json");
        yield* fs.writeFileString(existing, "{}");
        assert.equal(
          yield* broker.host.storage.validateSessionFile({
            threadId: THREAD_ID,
            path: existing,
            mustExist: true,
          }),
          yield* fs.realPath(existing),
        );
        const future = path.join(storage.sessionDirectory, "future.json");
        assert.equal(
          yield* broker.host.storage.validateSessionFile({
            threadId: THREAD_ID,
            path: future,
            mustExist: false,
          }),
          future,
        );
        yield* expectResourceFailure(
          broker.host.storage.validateSessionFile({
            threadId: THREAD_ID,
            path: future,
            mustExist: true,
          }),
        );
        yield* expectResourceFailure(
          broker.host.storage.validateSessionFile({
            threadId: THREAD_ID,
            path: path.join(config.baseDir, "outside.json"),
            mustExist: false,
          }),
        );
      }),
    ),
  );

  it.effect("rejects session-file symlink escapes", () =>
    runHostTest(
      Effect.gen(function* () {
        const fs = yield* FileSystem.FileSystem;
        const path = yield* Path.Path;
        const config = yield* ServerConfig;
        const broker = yield* makeHost();
        const storage = yield* broker.host.storage.prepareSession(THREAD_ID);
        const outside = path.join(config.baseDir, "outside.json");
        const linked = path.join(storage.sessionDirectory, "linked.json");
        yield* fs.writeFileString(outside, "secret");
        yield* fs.symlink(outside, linked);
        yield* expectResourceFailure(
          broker.host.storage.validateSessionFile({
            threadId: THREAD_ID,
            path: linked,
            mustExist: true,
          }),
        );
      }),
    ),
  );

  it.effect("materializes artifacts at a stable atomic path and blocks containment escapes", () =>
    runHostTest(
      Effect.gen(function* () {
        const fs = yield* FileSystem.FileSystem;
        const broker = yield* makeHost();
        const first = yield* broker.host.storage.materializeArtifact({
          key: "approval-extension",
          fileName: "server.py",
          content: "first",
        });
        const second = yield* broker.host.storage.materializeArtifact({
          key: "approval-extension",
          fileName: "server.py",
          content: new TextEncoder().encode("second"),
        });
        assert.equal(second, first);
        assert.equal(yield* fs.readFileString(first), "second");
        yield* expectResourceFailure(
          broker.host.storage.materializeArtifact({
            key: "../escape",
            fileName: "server.py",
            content: "bad",
          }),
        );
        yield* expectResourceFailure(
          broker.host.storage.materializeArtifact({
            key: "safe",
            fileName: "../escape.py",
            content: "bad",
          }),
        );
      }),
    ),
  );

  it.effect("reads attachments and rejects missing files and symlink escapes", () =>
    runHostTest(
      Effect.gen(function* () {
        const fs = yield* FileSystem.FileSystem;
        const path = yield* Path.Path;
        const config = yield* ServerConfig;
        const broker = yield* makeHost();
        const attachment = {
          type: "image",
          id: "thread-123-image",
          name: "image.png",
          mimeType: "image/png",
          sizeBytes: 3,
        } satisfies ChatAttachment;
        const attachmentPath = resolveAttachmentPath({
          attachmentsDir: config.attachmentsDir,
          attachment,
        });
        assert.isNotNull(attachmentPath);
        if (attachmentPath === null) return;
        const bytes = new Uint8Array([1, 2, 3]);
        yield* fs.writeFile(attachmentPath, bytes);
        const read = yield* broker.host.attachments.read(attachment);
        assert.deepStrictEqual(read.bytes, bytes);
        assert.equal(read.path, yield* fs.realPath(attachmentPath));

        yield* fs.remove(attachmentPath);
        yield* expectResourceFailure(broker.host.attachments.read(attachment));

        const outside = path.join(config.baseDir, "outside.png");
        yield* fs.writeFile(outside, bytes);
        yield* fs.symlink(outside, attachmentPath);
        yield* expectResourceFailure(broker.host.attachments.read(attachment));
      }),
    ),
  );
});
