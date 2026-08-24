# Coolify deployment

This directory deploys the published T3 Code server with the Codex CLI as an isolated Coolify application.

## Coolify settings

- Build pack: Docker Compose
- Base directory: `/deploy/coolify`
- Compose file: `/deploy/coolify/compose.yaml`
- Internal service port: `3773`
- Required runtime variable: `SHARED_PROJECTS_PATH`, set to the host path of the approved existing `projects` directory
- Generated URL: declared through `SERVICE_URL_T3CODE_3773` and `SERVICE_FQDN_T3CODE_3773`

T3 and Codex state use separate named volumes. Provider authentication is not committed. Run `codex login --device-auth` from a T3 terminal after deployment; `/home/node/.codex` persists the login.

The `serve` command intentionally has no positional workspace argument. `t3 serve` accepts server flags only; the container working directory is `/opt/data/projects`, and projects are selected through T3.
