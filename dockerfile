FROM rust:1.90-slim AS builder

RUN apt-get update && apt-get install -y libssl-dev pkg-config

WORKDIR /usr/src/

COPY . .

RUN cargo build --release

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
