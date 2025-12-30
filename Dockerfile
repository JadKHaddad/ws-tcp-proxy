FROM rust:1.90-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /usr/src/ws-tcp-proxy
COPY . .
RUN cargo build -p ws-tcp-proxy --release --target x86_64-unknown-linux-musl

FROM scratch
COPY --from=builder /usr/src/ws-tcp-proxy/target/x86_64-unknown-linux-musl/release/ws-tcp-proxy /ws-tcp-proxy
ENTRYPOINT ["/ws-tcp-proxy"]

# docker build -t jadkhaddad/ws-tcp-proxy:0.2.0 .