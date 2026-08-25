FROM node:24.13.1-bookworm-slim AS build

ENV DEBIAN_FRONTEND=noninteractive \
    COREPACK_HOME=/corepack \
    PNPM_HOME=/pnpm \
    PATH=/pnpm:$PATH \
    NODE_OPTIONS=--max-old-space-size=4096

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates g++ git make python3 \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps ./apps
COPY assets ./assets
COPY packages ./packages
COPY scripts ./scripts
COPY oxlint-plugin-t3code ./oxlint-plugin-t3code
COPY patches ./patches
COPY vite.config.ts t3.json tsconfig.base.json ./

RUN corepack enable \
 && corepack prepare pnpm@11.10.0 --activate \
 && pnpm install --frozen-lockfile \
 && pnpm exec vp run --filter @t3tools/web build \
 && pnpm exec vp run --filter t3 build \
 && pnpm --config.inject-workspace-packages=true --filter t3 deploy --prod /opt/t3

FROM node:24.13.1-bookworm-slim AS runtime

ARG CODEX_VERSION=0.149.1
ARG SOURCE_REF=deploy/prime-agent-rpc
ARG SOURCE_SHA=unknown

LABEL org.opencontainers.image.source="https://github.com/ricotrevisan/t3code" \
      org.opencontainers.image.revision="${SOURCE_SHA}" \
      io.t3code.source-ref="${SOURCE_REF}" \
      io.t3code.build-kind="source"

ENV DEBIAN_FRONTEND=noninteractive \
    NPM_CONFIG_UPDATE_NOTIFIER=false \
    NPM_CONFIG_FUND=false \
    NPM_CONFIG_AUDIT=false \
    T3CODE_HOME=/home/node/.t3 \
    T3CODE_SOURCE_REF=${SOURCE_REF} \
    T3CODE_SOURCE_SHA=${SOURCE_SHA} \
    T3CODE_BUILD_KIND=source

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git openssh-client tini \
 && rm -rf /var/lib/apt/lists/* \
 && npm install --global "@openai/codex@${CODEX_VERSION}" \
 && codex --version

COPY --from=build --chown=1000:1000 /opt/t3 /opt/t3

RUN mkdir -p /home/node/.t3 /home/node/.codex /opt/data/projects \
 && chown -R 1000:1000 /home/node/.t3 /home/node/.codex /opt/data/projects /opt/t3

USER 1000:1000
WORKDIR /opt/data/projects
EXPOSE 3773
ENTRYPOINT ["tini", "--"]
CMD ["node", "/opt/t3/dist/bin.mjs", "serve", "--host", "0.0.0.0", "--port", "3773", "--base-dir", "/home/node/.t3"]
