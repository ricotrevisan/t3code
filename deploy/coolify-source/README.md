# Coolify source deployment

This deployment builds T3 Code directly from the checked-out repository. In Coolify, use this directory's `compose.yml` as the Docker Compose file. Coolify's `SERVICE_FQDN_T3CODE_3773` magic variable provisions the application HTTPS domain and routes it to the container's only exposed port, `3773`; no host port is published.

Before deployment, define `SHARED_PROJECTS_PATH` as the verified absolute host directory that should be available to agents at `/opt/data/projects`. The compose file intentionally refuses to start without it rather than guessing a host path. Define `SOURCE_SHA` to the deployed Git commit when the deployment platform does not supply it automatically.

The named volumes are intentionally distinct and new:

- `t3_source_data` stores `/home/node/.t3`.
- `codex_source_data` stores `/home/node/.codex`.

Do not map either volume to an existing production T3 or Codex volume. Do not add a Hermes mount. The service and image run as UID/GID `1000:1000`, so ensure the shared projects directory is writable by that identity.
