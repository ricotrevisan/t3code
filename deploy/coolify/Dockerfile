FROM node:24-bookworm-slim

ARG T3_VERSION=0.0.33
ARG CODEX_VERSION=0.149.1

ENV DEBIAN_FRONTEND=noninteractive \
    NPM_CONFIG_UPDATE_NOTIFIER=false \
    NPM_CONFIG_FUND=false \
    NPM_CONFIG_AUDIT=false \
    T3CODE_HOME=/home/node/.t3

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      g++ \
      git \
      make \
      openssh-client \
      python3 \
      tini \
 && rm -rf /var/lib/apt/lists/*

RUN npm install --global "t3@${T3_VERSION}" "@openai/codex@${CODEX_VERSION}" \
 && t3 --help >/dev/null \
 && codex --version

RUN mkdir -p /home/node/.t3 /home/node/.codex /opt/data/projects \
 && chown -R 1000:1000 /home/node/.t3 /home/node/.codex /opt/data/projects

USER 1000:1000
WORKDIR /opt/data/projects

EXPOSE 3773
ENTRYPOINT ["tini", "--"]
CMD ["t3", "serve", "--host", "0.0.0.0", "--port", "3773", "--base-dir", "/home/node/.t3"]
