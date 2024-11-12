FROM rust:1.82-slim as builder

# Install OpenSSL development libraries
RUN apt-get update && apt-get install -y libssl-dev pkg-config

WORKDIR /usr/src/

COPY . .

RUN cargo build --release

FROM debian:bookworm-slim

# Install OpenSSL runtime libraries
RUN apt-get update && apt-get install -y libssl3 ca-certificates

WORKDIR /usr/app

COPY --from=builder /usr/src/assets/views /usr/app/assets/views
COPY --from=builder /usr/src/assets/static /usr/app/assets/static
COPY --from=builder /usr/src/config /usr/app/config
COPY --from=builder /usr/src/target/release/localtube-cli /usr/app/localtube-cli

ENTRYPOINT ["/usr/app/localtube-cli"]
