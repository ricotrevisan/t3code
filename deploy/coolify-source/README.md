# Coolify source deployment

This deployment builds T3 Code directly from the checked-out repository. In Coolify, use the repository-root `/compose.prime-agent-rpc.yml` as the Docker Compose file. Keeping the Compose file and build context at the repository root avoids nested base-directory ambiguity. Coolify's `SERVICE_FQDN_T3CODE_3773` magic variable provisions the application HTTPS domain and routes it to the container's only exposed port, `3773`; no host port is published.

The shared projects mount uses the verified Docker volume that backs `/opt/data` and mounts only its existing `projects` subdirectory at `/opt/data/projects`. The target Docker Compose implementation must support long-syntax `volume.subpath`; deployment should fail closed rather than falling back to a host bind or mounting all of `/opt/data`. Define `SOURCE_SHA` to the deployed Git commit when the deployment platform does not supply it automatically.

The named volumes are intentionally distinct and new:

- `t3_source_data` stores `/home/node/.t3`.
- `codex_source_data` stores `/home/node/.codex`.
- `prime_agent_source_data` stores mutable Prime Agent state in `/home/node/.prime`, including the coding-agent directory, kernel virtual environment, uv cache, and uv-managed Python installations.

Do not map any of these volumes to an existing production T3, Codex, or Prime Agent volume. Do not add a Hermes mount. The service and image run as UID/GID `1000:1000`, so ensure the shared projects directory is writable by that identity.

The Prime Agent CLI and `uv` executable are immutable, image-owned tools at `/opt/prime-agent` (exposed as `/usr/local/bin/prime-agent`) and `/usr/local/bin/uv`. They are never installed into `prime_agent_source_data`. Prime's first-use kernel environment and all other mutable state are instead created under `/home/node/.prime` on that volume; credentials are not baked into the image.

## Upgrading Prime Agent

Prime Agent is installed in the image with `npm ci` from the deployment-only manifest and integrity-locked dependency closure in `prime-agent-package.json` and `prime-agent-package-lock.json`. The top-level package is the official versioned release tarball, whose SHA-256 is also verified explicitly before installation; the mutable `install.sh` and live global npm resolution are not used. Runtime version checks are disabled.

To upgrade it, update the exact artifact URL in `prime-agent-package.json`, regenerate and review `prime-agent-package-lock.json`, and change `PRIME_AGENT_VERSION` and `PRIME_AGENT_SHA256` together in the Dockerfile using the checksum published for the matching official artifact. Also update the pinned CI version assertion. Validate the lock with a clean `npm ci`, rebuild the image, and confirm the build-time version check and CI runtime smoke tests pass before redeploying. Never update the CLI inside a running container; an in-container change is neither reviewed nor durable across image replacement. The `prime_agent_source_data` volume remains in place across image upgrades.

`uv` is independently pinned to the official Astral release `0.12.5`: `https://github.com/astral-sh/uv/releases/download/0.12.5/uv-x86_64-unknown-linux-gnu.tar.gz`, SHA-256 `68a509da24b06b4223a1c0175fb5eb5bc79342b76cbeff0cfe51ac3f5b17b6b2` from the adjacent official `.sha256` release asset. Upgrade `UV_VERSION`, `UV_SHA256`, and the CI assertion together.
