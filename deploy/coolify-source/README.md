# Coolify source deployment

This deployment builds T3 Code directly from the checked-out repository. In Coolify, use the repository-root `/compose.prime-agent-rpc.yml` as the Docker Compose file. Keeping the Compose file and build context at the repository root avoids nested base-directory ambiguity. Coolify's `SERVICE_FQDN_T3CODE_3773` magic variable provisions the application HTTPS domain and routes it to the container's only exposed port, `3773`; no host port is published.

The shared projects mount uses the verified Docker volume that backs `/opt/data` and mounts only its existing `projects` subdirectory at `/opt/data/projects`. The target Docker Compose implementation must support long-syntax `volume.subpath`; deployment should fail closed rather than falling back to a host bind or mounting all of `/opt/data`. Define `SOURCE_SHA` to the deployed Git commit when the deployment platform does not supply it automatically.

The named volumes are intentionally distinct and new:

- `t3_source_data` stores `/home/node/.t3`.
- `codex_source_data` stores `/home/node/.codex`.

Do not map either volume to an existing production T3 or Codex volume. Do not add a Hermes mount. The service and image run as UID/GID `1000:1000`, so ensure the shared projects directory is writable by that identity.
