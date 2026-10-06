# syntax=docker/dockerfile:1
# Builds the Rafi fullstack app (server binary + web bundle) with dx.

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
# The publishable key is public and must be identical in the wasm and server builds.
ARG CLERK_PUBLISHABLE_KEY
ENV CLERK_PUBLISHABLE_KEY=${CLERK_PUBLISHABLE_KEY}
RUN cd crates/app && dx bundle --web --release --fullstack

FROM debian:bookworm-slim AS runtime
# The same user as restate-rafi, which writes the same database file.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data && chown 65532:65532 /data
COPY --from=builder /app/target/dx/rafi/release/web/ /usr/local/app/
WORKDIR /usr/local/app
USER 65532:65532
ENV PORT=8080 \
    IP=0.0.0.0 \
    RAFI_DATABASE_URL=sqlite:///data/rafi.db
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["/usr/local/app/server"]
