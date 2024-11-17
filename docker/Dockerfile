FROM rust:1.82-bullseye  AS chef
WORKDIR sui
ARG GIT_REVISION
ENV GIT_REVISION=$GIT_REVISION
RUN apt-get update && apt-get install -y cmake clang

# Build and cache all dependencies.
FROM chef AS builder
WORKDIR /
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

# Production Image
FROM debian:bookworm AS runtime
RUN apt-get update && apt-get install -y libpq5 libjemalloc-dev ca-certificates curl wget haproxy
COPY --from=builder /target/release/sui_gas_pool /usr/local/bin
COPY testnet-prod.yaml /usr/local/bin
COPY haproxy.cfg /etc/haproxy/haproxy.cfg
COPY promtail-config.yml /etc/promtail/config.yml    

# libssl1.1 is not available in bookworm, so we need to install it from buster
RUN echo "deb http://deb.debian.org/debian buster main" >> /etc/apt/sources.list && apt-get update && apt-get install -y libssl1.1

# Download and install Promtail
RUN curl -L -o /usr/local/bin/promtail.zip https://github.com/grafana/loki/releases/download/v2.8.0/promtail-linux-amd64.zip \
    && unzip /usr/local/bin/promtail.zip -d /usr/local/bin/ \
    && rm /usr/local/bin/promtail.zip \
    && chmod +x /usr/local/bin/promtail

ARG BUILD_DATE
ARG GIT_REVISION
LABEL build-date=$BUILD_DATE
LABEL git-revision=$GIT_REVISION

EXPOSE 8084
ENV PORT 8084
