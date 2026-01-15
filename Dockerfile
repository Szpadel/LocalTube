# syntax=docker/dockerfile:1.7-labs

FROM rust:1.90-slim AS chef
WORKDIR /usr/src/
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
  && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY migration/Cargo.toml migration/Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /usr/src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bins

FROM rust:1.90-slim AS cargo
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    build-essential \
    cmake \
    python3 \
    pkg-config \
    libssl-dev \
    clang \
    libclang-dev \
    llvm-dev \
    libsqlite3-dev \
  && rm -rf /var/lib/apt/lists/*
RUN cargo install deno --locked

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tini \
    curl \
    ffmpeg \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/app

COPY --from=builder /usr/src/assets /usr/app/assets
COPY --from=builder /usr/src/config /usr/app/config
COPY --from=builder /usr/src/target/release/localtube-cli /usr/app/localtube-cli
COPY --from=cargo /usr/local/cargo/bin/deno /usr/local/bin/deno

# Sanity check
RUN deno --version && /usr/app/localtube-cli --version

ENTRYPOINT ["tini", "--", "/usr/app/localtube-cli"]
