# The console: the pages the owner opens, rendered on the server, and the
# relay that passes the browser's API calls to the backend.
# Built from the repository root: `docker build -f frontend/Dockerfile .`

FROM node:24-trixie-slim AS build
ARG BUF_VERSION=1.75.0
ARG PNPM_VERSION=12.21.0
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && curl -fsSL -o /usr/local/bin/buf \
      "https://github.com/bufbuild/buf/releases/download/v${BUF_VERSION}/buf-Linux-$(uname -m)" \
 && chmod +x /usr/local/bin/buf \
 && npm install -g "pnpm@${PNPM_VERSION}"
WORKDIR /src
COPY proto proto
COPY frontend/package.json frontend/pnpm-lock.yaml frontend/
RUN cd frontend && pnpm install --frozen-lockfile
COPY frontend frontend
RUN cd frontend && pnpm gen && pnpm build

FROM node:24-trixie-slim
COPY --from=build /src/frontend/.output /app
USER node
ENV HOST=0.0.0.0 \
    PORT=3000 \
    FAMULEN_API_URL=http://backend:7790
EXPOSE 3000
CMD ["node", "/app/server/index.mjs"]
