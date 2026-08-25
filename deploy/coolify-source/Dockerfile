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
ARG PRIME_AGENT_VERSION=0.8.0
ARG PRIME_AGENT_SHA256=f5b0093c7e0fddb73f94773d74383585456adfa84f12a4082d3098f23bb8fab6
ARG UV_VERSION=0.12.5
ARG UV_SHA256=68a509da24b06b4223a1c0175fb5eb5bc79342b76cbeff0cfe51ac3f5b17b6b2
ARG SOURCE_REF=deploy/prime-agent-rpc
ARG SOURCE_SHA=unknown

LABEL org.opencontainers.image.source="https://github.com/ricotrevisan/t3code" \
      org.opencontainers.image.revision="${SOURCE_SHA}" \
      io.t3code.source-ref="${SOURCE_REF}" \
      io.t3code.build-kind="source"

ENV DEBIAN_FRONTEND=noninteractive \
    HOME=/home/node \
    NPM_CONFIG_UPDATE_NOTIFIER=false \
    NPM_CONFIG_FUND=false \
    NPM_CONFIG_AUDIT=false \
    PI_SKIP_VERSION_CHECK=1 \
    PRIME_AGENT_CODING_AGENT_DIR=/home/node/.prime/agent \
    PRIME_AGENT_KERNEL_VENV=/home/node/.prime/agent/kernel-venv \
    T3CODE_HOME=/home/node/.t3 \
    T3CODE_SOURCE_REF=${SOURCE_REF} \
    T3CODE_SOURCE_SHA=${SOURCE_SHA} \
    T3CODE_BUILD_KIND=source \
    UV_CACHE_DIR=/home/node/.prime/uv/cache \
    UV_PYTHON_INSTALL_DIR=/home/node/.prime/uv/python

COPY deploy/coolify-source/prime-agent-package.json /opt/prime-agent/package.json
COPY deploy/coolify-source/prime-agent-package-lock.json /opt/prime-agent/package-lock.json

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git openssh-client tini \
 && rm -rf /var/lib/apt/lists/* \
 && npm install --global "@openai/codex@${CODEX_VERSION}" \
 && codex --version \
 && curl --fail --location --silent --show-error \
      "https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/uv-x86_64-unknown-linux-gnu.tar.gz" \
      --output /tmp/uv.tar.gz \
 && printf '%s  %s\n' "${UV_SHA256}" /tmp/uv.tar.gz | sha256sum --check --strict \
 && tar --extract --gzip --file /tmp/uv.tar.gz --strip-components=1 \
      --directory /usr/local/bin uv-x86_64-unknown-linux-gnu/uv \
 && rm /tmp/uv.tar.gz \
 && test "$(uv --version | cut -d ' ' -f 1-2)" = "uv ${UV_VERSION}" \
 && curl --fail --location --silent --show-error \
      "https://pub-728493de92a943e2a9b2d17b4719f318.r2.dev/releases/v${PRIME_AGENT_VERSION}/prime-agent-${PRIME_AGENT_VERSION}.tgz" \
      --output /tmp/prime-agent.tgz \
 && printf '%s  %s\n' "${PRIME_AGENT_SHA256}" /tmp/prime-agent.tgz | sha256sum --check --strict \
 && PRIME_AGENT_BOOTSTRAP_KERNEL_ON_INSTALL=0 \
      PRIME_AGENT_BOOTSTRAP_TOOLS_ON_INSTALL=0 \
      npm ci --omit=dev --prefix /opt/prime-agent \
 && ln -s /opt/prime-agent/node_modules/.bin/prime-agent /usr/local/bin/prime-agent \
 && rm /tmp/prime-agent.tgz \
 && test "$(prime-agent --version 2>&1)" = "${PRIME_AGENT_VERSION}" \
 && test ! -e /root/.prime

COPY --from=build --chown=1000:1000 /opt/t3 /opt/t3

RUN mkdir -p /home/node/.t3 /home/node/.codex /home/node/.prime /opt/data/projects \
 && chown -R 1000:1000 /home/node/.t3 /home/node/.codex /home/node/.prime /opt/data/projects /opt/t3

USER 1000:1000
WORKDIR /opt/data/projects
EXPOSE 3773
ENTRYPOINT ["tini", "--"]
CMD ["node", "/opt/t3/dist/bin.mjs", "serve", "--host", "0.0.0.0", "--port", "3773", "--base-dir", "/home/node/.t3"]
