FROM rust:1.98-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY server ./server
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked \
    && cp /build/target/release/lumichat /build/lumichat

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --create-home lumichat \
    && mkdir -p /app/data/uploads \
    && chown -R lumichat:lumichat /app
WORKDIR /app
COPY --from=builder /build/lumichat /usr/local/bin/lumichat
COPY web ./web
USER lumichat
ENV LUMICHAT_BIND=0.0.0.0:8080 \
    LUMICHAT_DATABASE=/app/data/lumichat.db \
    LUMICHAT_UPLOADS=/app/data/uploads \
    LUMICHAT_WEB=/app/web
EXPOSE 8080
VOLUME ["/app/data"]
ENTRYPOINT ["/usr/local/bin/lumichat"]
