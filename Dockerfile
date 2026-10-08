# syntax=docker/dockerfile:1
# Builds Rafi: the web app (admin, projector, verification page) with dx, and the native
# server (attendee pages, JSON APIs) with cargo. The server serves the web app's files.

FROM rust:1-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN rustup target add wasm32-unknown-unknown \
    && cargo install dioxus-cli@0.7.10 --locked
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --features server --recipe-path recipe.json -p rafi
COPY . .
# The publishable key is public; the web app bakes it in (without it the admin runs with dev auth).
ARG CLERK_PUBLISHABLE_KEY
ENV CLERK_PUBLISHABLE_KEY=${CLERK_PUBLISHABLE_KEY}
# -> target/dx/rafi/release/web/public
RUN cd crates/app && dx bundle --platform web --release
RUN cargo build --release --locked -p rafi --features server

FROM debian:bookworm-slim AS runtime
# The same user as restate-rafi, which writes the same database file.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data && chown 65532:65532 /data
COPY --from=builder /app/target/dx/rafi/release/web/public/ /usr/local/app/public/
COPY --from=builder /app/target/release/rafi /usr/local/app/server
WORKDIR /usr/local/app
USER 65532:65532
ENV PORT=8080 \
    IP=0.0.0.0 \
    RAFI_PUBLIC_DIR=/usr/local/app/public \
    RAFI_DATABASE_URL=sqlite:///data/rafi.db
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["/usr/local/app/server"]
