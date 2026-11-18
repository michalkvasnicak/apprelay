# syntax=docker/dockerfile:1
FROM node:24-bookworm-slim AS web

WORKDIR /build
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/server/web/package.json ./apps/server/web/package.json
RUN pnpm install --filter @apprelay/web --frozen-lockfile --ignore-scripts
COPY apps/server/web ./apps/server/web
ARG VITE_APPRELAY_APP_SCHEME
ARG VITE_SENTRY_DSN
ARG VITE_SENTRY_ENVIRONMENT=production
ARG SENTRY_ORG
ARG SENTRY_PROJECT
ARG SENTRY_RELEASE
# Keep upload credentials out of image layers and build arguments. Vite requires
# the token only when a DSN is configured, ensuring those builds upload maps.
RUN --mount=type=secret,id=SENTRY_AUTH_TOKEN,env=SENTRY_AUTH_TOKEN \
    pnpm --filter @apprelay/web build

FROM getsentry/sentry-cli:3.8.1 AS sentry-cli

FROM rust:1.92-bookworm AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        binutils ca-certificates clang cmake git libprotobuf-dev libssl-dev pkg-config protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

COPY --from=sentry-cli /bin/sentry-cli /usr/local/bin/sentry-cli

WORKDIR /build
COPY apps/server/Cargo.toml apps/server/Cargo.lock apps/server/build.rs ./
COPY apps/server/src ./src
COPY apps/server/migrations ./migrations

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --locked --release --bins --features sentry-debug-images \
    && mkdir /out /symbols \
    && cp target/release/apprelay-server target/release/vpn-agent \
        target/release/pairing-worker target/release/notification-worker \
        target/release/install-worker /out/ \
    && for binary in /out/*; do \
        objcopy --only-keep-debug "$binary" "/symbols/$(basename "$binary").debug" || exit 1; \
        objcopy --strip-debug --strip-unneeded "$binary" || exit 1; \
    done

ARG SENTRY_ORG
ARG SENTRY_BACKEND_PROJECT
ARG SENTRY_RELEASE
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=secret,id=SENTRY_AUTH_TOKEN,env=SENTRY_AUTH_TOKEN \
    if [ -n "$SENTRY_BACKEND_PROJECT" ]; then \
        : "${SENTRY_ORG:?SENTRY_ORG is required to upload backend debug files}"; \
        : "${SENTRY_AUTH_TOKEN:?SENTRY_AUTH_TOKEN is required to upload backend debug files}"; \
        sentry-cli debug-files upload --org "$SENTRY_ORG" \
            --project "$SENTRY_BACKEND_PROJECT" --include-sources --wait /out /symbols; \
    fi

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
COPY --from=web /build/apps/server/web/dist/ /opt/apprelay/web/
ARG SENTRY_RELEASE
ENV APPRELAY_WEB_DIR=/opt/apprelay/web \
    SENTRY_RELEASE=${SENTRY_RELEASE}
COPY --chmod=0755 apps/server/docker/vpn-agent-entrypoint.sh /usr/local/bin/vpn-agent-entrypoint
# Docker drops effective capabilities for non-root users. This file capability
# lets only the pairing executable acquire NET_RAW within its bounding set.
RUN setcap cap_net_raw=ep /usr/local/bin/pairing-worker

USER 10001:10001
CMD ["apprelay-server"]
