# syntax=docker/dockerfile:1

############################
# Build stage
############################
FROM rust:1-bookworm AS builder
WORKDIR /app

# Source. Askama compiles templates/ at build time, so the dir must be present here.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates

# BuildKit cache mounts keep the cargo registry + target dir out of the final image
# while still caching between builds. The binary is copied out to /share-secret.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked \
    && cp target/release/share-secret /share-secret

############################
# Runtime stage
############################
FROM debian:bookworm-slim AS runtime
WORKDIR /app

# Non-root runtime user. SQLite is bundled (libsqlite3-sys), so no extra system libs.
RUN useradd --uid 10001 --user-group --no-create-home --shell /usr/sbin/nologin app

# Static assets are served from ./static at runtime via ServeDir.
COPY static ./static
COPY --from=builder /share-secret /usr/local/bin/share-secret

ENV BIND_ADDR=0.0.0.0:3000
EXPOSE 3000

USER app
ENTRYPOINT ["share-secret"]
