# syntax=docker/dockerfile:1
FROM rust:1.92-bookworm AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates clang cmake git libssl-dev pkg-config protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY migrations ./migrations

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --locked --release --bins \
    && mkdir /out \
    && cp target/release/apprelay-server target/release/vpn-agent \
        target/release/pairing-worker target/release/notification-worker /out/

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl iproute2 iptables libcap2-bin libssl3 libstdc++6 \
        util-linux wireguard-tools \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 apprelay \
    && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin apprelay \
    && install -d -m 0755 /run/apprelay

COPY --from=build /out/ /usr/local/bin/
COPY --chmod=0755 docker/vpn-agent-entrypoint.sh /usr/local/bin/vpn-agent-entrypoint
# Docker drops effective capabilities for non-root users. This file capability
# lets only the pairing executable acquire NET_RAW within its bounding set.
RUN setcap cap_net_raw=ep /usr/local/bin/pairing-worker

USER 10001:10001
CMD ["apprelay-server"]
