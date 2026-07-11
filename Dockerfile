# Agent Manager — headless HTTP server for Docker / k8s.
# Intentionally has NO coding-agent CLIs (claude / grok / codex).
# Enable agents only in the Windows app on a machine where those CLIs are installed.

# syntax=docker/dockerfile:1

FROM rust:1.85-bookworm AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache deps
COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY assets ./assets
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release 2>/dev/null || true \
    && rm -rf src

COPY src ./src
COPY static ./static
RUN touch src/main.rs && cargo build --release \
    && strip target/release/agent-manager

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates \
      git \
      curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 --shell /usr/sbin/nologin agentmgr

WORKDIR /app
COPY --from=builder /src/target/release/agent-manager /usr/local/bin/agent-manager

# Data dirs (mount repos + config here)
RUN mkdir -p /data/repos /data/config /data/logs \
    && chown -R agentmgr:agentmgr /data /app

USER agentmgr

ENV AGENT_MANAGER_ROOT=/data/repos \
    AGENT_MANAGER_HOST=0.0.0.0 \
    AGENT_MANAGER_PORT=7878 \
    AGENT_MANAGER_CONFIG=/data/config/agent-manager.config.json \
    AGENT_MANAGER_LOG_FILE=/data/logs/agent-manager.log \
    AGENT_MANAGER_LOG_LEVEL=info \
    AGENT_MANAGER_ALLOW_AGENTS=0

EXPOSE 7878

HEALTHCHECK --interval=30s --timeout=5s --start-period=40s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:${AGENT_MANAGER_PORT}/api/health" >/dev/null || exit 1

# Server-only: no window, no agent CLIs in image
ENTRYPOINT ["agent-manager"]
CMD ["--mode", "server", "--no-open", "--host", "0.0.0.0"]
