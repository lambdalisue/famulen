# The backend: the API, the secretary's session and everything it keeps.
# Built from the repository root: `docker build -t famulen .`

FROM rust:1.100.0-slim-trixie AS build
ARG BUF_VERSION=1.75.0
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && curl -fsSL -o /usr/local/bin/buf \
      "https://github.com/bufbuild/buf/releases/download/v${BUF_VERSION}/buf-Linux-$(uname -m)" \
 && chmod +x /usr/local/bin/buf
WORKDIR /src
# rust-toolchain.toml is left out: the image already has that toolchain, and
# the file would pull in the editor components on top.
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY proto proto
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p famulen \
 && cp target/release/famulen /usr/local/bin/famulen

# Node is here for Claude Code, which the secretary runs as.
FROM node:24-trixie-slim
ARG CLAUDE_CODE_VERSION=2.1.330
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git tini bubblewrap socat \
 && rm -rf /var/lib/apt/lists/* \
 && npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
 && mkdir -p /data /home/node/.claude \
 && chown node:node /data /home/node/.claude
COPY --from=build /usr/local/bin/famulen /usr/local/bin/famulen
USER node
ENV FAMULEN_DATA_DIR=/data \
    FAMULEN_LISTEN=0.0.0.0:7790
VOLUME ["/data", "/home/node/.claude"]
EXPOSE 7790
ENTRYPOINT ["tini", "--", "famulen"]
