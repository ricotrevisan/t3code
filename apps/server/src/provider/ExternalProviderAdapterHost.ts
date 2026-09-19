import type {
  ProviderAdapterHostV2,
  ProviderAdapterStorageV2,
  ProviderAdapterHostResourceOperation,
} from "@t3tools/provider-adapter";
import {
  ProviderAdapterHostResourceError,
  T3_PROVIDER_ADAPTER_HOST_PROTOCOL_VERSION,
} from "@t3tools/provider-adapter";
import type { ProviderAdapterPackageId, ProviderInstanceId } from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";
import type * as Scope from "effect/Scope";
import type * as ChildProcessSpawner from "effect/unstable/process/ChildProcessSpawner";

import { resolveAttachmentPath } from "../attachmentStore.ts";
import { ServerConfig } from "../config.ts";
import {
  makeExternalProviderProcessSupervisor,
  type ExternalProviderProcessSupervisor,
} from "./ExternalProviderProcessSupervisor.ts";

const resourceError = (
  operation: ProviderAdapterHostResourceOperation,
  detail: string,
  cause?: unknown,
) =>
  new ProviderAdapterHostResourceError({
    operation,
    detail,
    ...(cause === undefined ? {} : { cause }),
  });

function isSinglePathSegment(value: string): boolean {
  return (
    value.length > 0 &&
    value !== "." &&
    value !== ".." &&
    !value.includes("/") &&
    !value.includes("\\") &&
    !value.includes("\0")
  );
}

function isPathWithin(
  path: Path.Path,
  root: string,
  candidate: string,
  allowRoot = false,
): boolean {
  const relative = path.relative(path.resolve(root), path.resolve(candidate));
  return (
    (allowRoot && relative === "") ||
    (relative !== "" && !relative.startsWith("..") && !path.isAbsolute(relative))
  );
}

export interface ExternalProviderAdapterHostV2Input {
  readonly packageId: ProviderAdapterPackageId;
  readonly instanceId: ProviderInstanceId;
  /** A legacy, shared storage root. It must be one path segment below T3 home. */
  readonly storageKey?: string | undefined;
}

export interface ExternalProviderAdapterHostV2 {
  readonly host: ProviderAdapterHostV2;
  readonly unexpectedExits: ExternalProviderProcessSupervisor["unexpectedExits"];
}

export const makeExternalProviderAdapterHostV2 = Effect.fn("makeExternalProviderAdapterHostV2")(
  function* (
    input: ExternalProviderAdapterHostV2Input,
  ): Effect.fn.Return<
    ExternalProviderAdapterHostV2,
    ProviderAdapterHostResourceError,
    | FileSystem.FileSystem
    | Path.Path
    | ServerConfig
    | Scope.Scope
    | ChildProcessSpawner.ChildProcessSpawner
  > {
    const fileSystem = yield* FileSystem.FileSystem;
    const path = yield* Path.Path;
    const config = yield* ServerConfig;

    const packageId = String(input.packageId);
    const instanceId = String(input.instanceId);
    if (!isSinglePathSegment(packageId) || !isSinglePathSegment(instanceId)) {
      return yield* resourceError(
        "storage.configure",
        "Provider package and instance ids must be single path segments.",
      );
    }
    if (input.storageKey !== undefined && !isSinglePathSegment(input.storageKey)) {
      return yield* resourceError(
        "storage.configure",
        "The provider storage key must be a single path segment.",
      );
    }

    const baseDirectory = path.resolve(config.baseDir);
    const storageRoot =
      input.storageKey === undefined
        ? path.resolve(baseDirectory, "provider-adapters", packageId, instanceId)
        : path.resolve(baseDirectory, input.storageKey);
    if (!isPathWithin(path, baseDirectory, storageRoot)) {
      return yield* resourceError(
        "storage.configure",
        "The provider storage root must be inside the server base directory.",
      );
    }

    yield* fileSystem
      .makeDirectory(storageRoot, { recursive: true })
      .pipe(
        Effect.mapError((cause) =>
          resourceError("storage.configure", "Could not prepare provider storage.", cause),
        ),
      );
    const [realBaseDirectory, realStorageRoot] = yield* Effect.all([
      fileSystem.realPath(baseDirectory),
      fileSystem.realPath(storageRoot),
    ]).pipe(
      Effect.mapError((cause) =>
        resourceError("storage.configure", "Could not resolve provider storage.", cause),
      ),
    );
    if (!isPathWithin(path, realBaseDirectory, realStorageRoot)) {
      return yield* resourceError(
        "storage.configure",
        "The provider storage root resolves outside the server base directory.",
      );
    }

    const requireSegment = (
      operation: ProviderAdapterHostResourceOperation,
      kind: string,
      value: string,
    ) =>
      isSinglePathSegment(value)
        ? Effect.succeed(value)
        : Effect.fail(resourceError(operation, `${kind} must be a single path segment.`));

    const prepareSession: ProviderAdapterStorageV2["prepareSession"] = (threadId) =>
      Effect.gen(function* () {
        const segment = yield* requireSegment(
          "storage.prepareSession",
          "Thread id",
          String(threadId),
        );
        const sessionDirectory = path.resolve(storageRoot, "sessions", segment);
        yield* fileSystem
          .makeDirectory(sessionDirectory, { recursive: true })
          .pipe(
            Effect.mapError((cause) =>
              resourceError(
                "storage.prepareSession",
                "Could not prepare the provider session directory.",
                cause,
              ),
            ),
          );
        const realSessionDirectory = yield* fileSystem
          .realPath(sessionDirectory)
          .pipe(
            Effect.mapError((cause) =>
              resourceError(
                "storage.prepareSession",
                "Could not resolve the provider session directory.",
                cause,
              ),
            ),
          );
        if (!isPathWithin(path, realStorageRoot, realSessionDirectory)) {
          return yield* resourceError(
            "storage.prepareSession",
            "The provider session directory resolves outside provider storage.",
          );
        }
        return { sessionDirectory, sharedDirectory: storageRoot };
      });

    const validateSessionFile: ProviderAdapterStorageV2["validateSessionFile"] = (request) =>
      Effect.gen(function* () {
        const segment = yield* requireSegment(
          "storage.validateSessionFile",
          "Thread id",
          String(request.threadId),
        );
        const sessionDirectory = path.resolve(storageRoot, "sessions", segment);
        const requestedPath = path.resolve(request.path);
        const realSessionDirectory = yield* fileSystem
          .realPath(sessionDirectory)
          .pipe(
            Effect.mapError((cause) =>
              resourceError(
                "storage.validateSessionFile",
                "Could not resolve this thread's session directory.",
                cause,
              ),
            ),
          );
        const exists = yield* fileSystem
          .exists(requestedPath)
          .pipe(
            Effect.mapError((cause) =>
              resourceError(
                "storage.validateSessionFile",
                "Could not inspect the session file.",
                cause,
              ),
            ),
          );
        if (!exists && request.mustExist) {
          return yield* resourceError(
            "storage.validateSessionFile",
            "The provider session file does not exist.",
          );
        }

        if (exists) {
          const realFile = yield* fileSystem
            .realPath(requestedPath)
            .pipe(
              Effect.mapError((cause) =>
                resourceError(
                  "storage.validateSessionFile",
                  "Could not resolve the session file.",
                  cause,
                ),
              ),
            );
          if (!isPathWithin(path, realSessionDirectory, realFile)) {
            return yield* resourceError(
              "storage.validateSessionFile",
              "The session file resolves outside this thread's session directory.",
            );
          }
          const info = yield* fileSystem
            .stat(realFile)
            .pipe(
              Effect.mapError((cause) =>
                resourceError(
                  "storage.validateSessionFile",
                  "Could not inspect the session file.",
                  cause,
                ),
              ),
            );
          if (info.type !== "File") {
            return yield* resourceError(
              "storage.validateSessionFile",
              "The provider session path is not a file.",
            );
          }
          return realFile;
        }

        let ancestor = path.dirname(requestedPath);
        while (!(yield* fileSystem.exists(ancestor).pipe(Effect.orElseSucceed(() => false)))) {
          const parent = path.dirname(ancestor);
          if (parent === ancestor) {
            return yield* resourceError(
              "storage.validateSessionFile",
              "Could not find an existing parent for the session file.",
            );
          }
          ancestor = parent;
        }
        const realAncestor = yield* fileSystem
          .realPath(ancestor)
          .pipe(
            Effect.mapError((cause) =>
              resourceError(
                "storage.validateSessionFile",
                "Could not resolve the session file parent.",
                cause,
              ),
            ),
          );
        if (!isPathWithin(path, realSessionDirectory, realAncestor, true)) {
          return yield* resourceError(
            "storage.validateSessionFile",
            "The session file parent resolves outside this thread's session directory.",
          );
        }
        return requestedPath;
      });

    const materializeArtifact: ProviderAdapterStorageV2["materializeArtifact"] = (request) =>
      Effect.gen(function* () {
        const key = yield* requireSegment(
          "storage.materializeArtifact",
          "Artifact key",
          request.key,
        );
        const fileName = yield* requireSegment(
          "storage.materializeArtifact",
          "Artifact file name",
          request.fileName,
        );
        const artifactDirectory = path.resolve(storageRoot, "artifacts", key);
        const artifactPath = path.resolve(artifactDirectory, fileName);
        if (!isPathWithin(path, storageRoot, artifactPath)) {
          return yield* resourceError(
            "storage.materializeArtifact",
            "The artifact path is outside provider storage.",
          );
        }
        yield* fileSystem
          .makeDirectory(artifactDirectory, { recursive: true })
          .pipe(
            Effect.mapError((cause) =>
              resourceError(
                "storage.materializeArtifact",
                "Could not prepare the artifact directory.",
                cause,
              ),
            ),
          );
        const realArtifactDirectory = yield* fileSystem
          .realPath(artifactDirectory)
          .pipe(
            Effect.mapError((cause) =>
              resourceError(
                "storage.materializeArtifact",
                "Could not resolve the artifact directory.",
                cause,
              ),
            ),
          );
        if (!isPathWithin(path, realStorageRoot, realArtifactDirectory)) {
          return yield* resourceError(
            "storage.materializeArtifact",
            "The artifact directory resolves outside provider storage.",
          );
        }
        const targetExists = yield* fileSystem
          .exists(artifactPath)
          .pipe(
            Effect.mapError((cause) =>
              resourceError(
                "storage.materializeArtifact",
                "Could not inspect the artifact path.",
                cause,
              ),
            ),
          );
        if (targetExists) {
          const realTarget = yield* fileSystem
            .realPath(artifactPath)
            .pipe(
              Effect.mapError((cause) =>
                resourceError(
                  "storage.materializeArtifact",
                  "Could not resolve the artifact path.",
                  cause,
                ),
              ),
            );
          if (!isPathWithin(path, realStorageRoot, realTarget)) {
            return yield* resourceError(
              "storage.materializeArtifact",
              "The artifact path resolves outside provider storage.",
            );
          }
        }

        yield* Effect.scoped(
          Effect.gen(function* () {
            const tempDirectory = yield* fileSystem.makeTempDirectoryScoped({
              directory: realArtifactDirectory,
              prefix: `${fileName}.`,
            });
            const tempPath = path.join(tempDirectory, "contents.tmp");
            if (typeof request.content === "string") {
              yield* fileSystem.writeFileString(tempPath, request.content);
            } else {
              yield* fileSystem.writeFile(tempPath, request.content);
            }
            yield* fileSystem.rename(tempPath, artifactPath);
          }),
        ).pipe(
          Effect.mapError((cause) =>
            resourceError(
              "storage.materializeArtifact",
              "Could not write the provider artifact.",
              cause,
            ),
          ),
        );
        return artifactPath;
      });

    const resolveCwd: ProviderAdapterHostV2["workspaces"]["resolveCwd"] = (requested) =>
      Effect.gen(function* () {
        const candidate = requested === undefined ? config.cwd : requested.trim();
        if (candidate.length === 0) {
          return yield* resourceError(
            "workspaces.resolveCwd",
            "The workspace cwd must not be empty.",
          );
        }
        const resolved = path.resolve(candidate);
        const info = yield* fileSystem
          .stat(resolved)
          .pipe(
            Effect.mapError((cause) =>
              resourceError("workspaces.resolveCwd", "The workspace cwd does not exist.", cause),
            ),
          );
        if (info.type !== "Directory") {
          return yield* resourceError(
            "workspaces.resolveCwd",
            "The workspace cwd is not a directory.",
          );
        }
        return yield* fileSystem
          .realPath(resolved)
          .pipe(
            Effect.mapError((cause) =>
              resourceError("workspaces.resolveCwd", "Could not resolve the workspace cwd.", cause),
            ),
          );
      });

    const readAttachment: ProviderAdapterHostV2["attachments"]["read"] = (attachment) =>
      Effect.gen(function* () {
        const attachmentPath = resolveAttachmentPath({
          attachmentsDir: config.attachmentsDir,
          attachment,
        });
        if (attachmentPath === null) {
          return yield* resourceError("attachments.read", "The attachment path is invalid.");
        }
        const realAttachmentsDirectory = yield* fileSystem
          .realPath(config.attachmentsDir)
          .pipe(
            Effect.mapError((cause) =>
              resourceError("attachments.read", "Could not resolve attachment storage.", cause),
            ),
          );
        const realAttachmentPath = yield* fileSystem
          .realPath(attachmentPath)
          .pipe(
            Effect.mapError((cause) =>
              resourceError("attachments.read", "The attachment file does not exist.", cause),
            ),
          );
        if (!isPathWithin(path, realAttachmentsDirectory, realAttachmentPath)) {
          return yield* resourceError(
            "attachments.read",
            "The attachment resolves outside attachment storage.",
          );
        }
        const info = yield* fileSystem
          .stat(realAttachmentPath)
          .pipe(
            Effect.mapError((cause) =>
              resourceError("attachments.read", "Could not inspect the attachment file.", cause),
            ),
          );
        if (info.type !== "File") {
          return yield* resourceError("attachments.read", "The attachment path is not a file.");
        }
        const bytes = yield* fileSystem
          .readFile(realAttachmentPath)
          .pipe(
            Effect.mapError((cause) =>
              resourceError("attachments.read", "Could not read the attachment file.", cause),
            ),
          );
        return { bytes, path: realAttachmentPath };
      });

    const processSupervisor = yield* makeExternalProviderProcessSupervisor();
    return {
      host: {
        protocolVersion: T3_PROVIDER_ADAPTER_HOST_PROTOCOL_VERSION,
        processes: processSupervisor.processes,
        workspaces: { resolveCwd },
        storage: { prepareSession, validateSessionFile, materializeArtifact },
        attachments: { read: readAttachment },
      },
      unexpectedExits: processSupervisor.unexpectedExits,
    };
  },
);
