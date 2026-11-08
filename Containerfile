FROM docker.io/library/node:22-bookworm-slim AS frontend
WORKDIR /src
COPY package.json package-lock.json vite.config.js ./
RUN npm ci
COPY web/ ./web/
RUN npm run build

FROM docker.io/library/rust:1.99-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
COPY --from=frontend /src/web/dist/ ./web/dist/
RUN cargo build --release --locked

FROM docker.io/library/debian:bookworm-slim AS runtime
# chromium renders PDFs; poppler-utils provides pdftotext (text-layer check) and
# pdftoppm (page previews); fonts-dejavu-core covers glyphs Liberation lacks.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
       chromium poppler-utils fonts-liberation fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 documents \
    && useradd --uid 10001 --gid documents --no-create-home documents \
    && mkdir -p /data \
    && chown documents:documents /data
COPY config/pdf-fonts.conf /etc/fonts/conf.d/99-document-pdf-fonts.conf
RUN fc-cache -f \
    && test "$(fc-match -f '%{family}' sans-serif)" = 'Liberation Sans' \
    && test "$(fc-match -f '%{family}' serif)" = 'Liberation Serif' \
    && test "$(fc-match -f '%{family}' monospace)" = 'Liberation Mono'
COPY --from=build /src/target/release/document-templating-mcp /usr/local/bin/document-templating-mcp
ENV DOCUMENT_TEMPLATING_MCP_TRANSPORT=http \
    DOCUMENT_TEMPLATING_MCP_HTTP_ADDR=0.0.0.0:8080 \
    DOCUMENT_TEMPLATING_MCP_DATA_DIR=/data \
    DOCUMENT_TEMPLATING_MCP_CHROMIUM=/usr/bin/chromium \
    DOCUMENT_TEMPLATING_MCP_CHROMIUM_NO_SANDBOX=1 \
    HOME=/tmp \
    XDG_CONFIG_HOME=/tmp/.config \
    XDG_CACHE_HOME=/tmp/.cache
USER documents
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/document-templating-mcp"]
