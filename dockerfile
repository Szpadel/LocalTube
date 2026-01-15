FROM rust:1.90-slim AS builder

RUN apt-get update && apt-get install -y libssl-dev pkg-config

WORKDIR /usr/src/

COPY Cargo.toml Cargo.lock ./
COPY migration/Cargo.toml migration/Cargo.toml
RUN mkdir -p src/bin migration/src \
  && printf 'fn main() {}\n' > src/bin/main.rs \
  && printf 'fn main() {}\n' > src/bin/tool.rs \
  && touch src/lib.rs migration/src/lib.rs
RUN cargo build --release --bins

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
