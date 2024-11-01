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
RUN apt-get update && apt-get install -y libpq5 libjemalloc-dev ca-certificates curl 
COPY --from=builder /target/release/sui_gas_pool /usr/local/bin
COPY testnet-prod.yaml /usr/local/bin


ARG BUILD_DATE
ARG GIT_REVISION
LABEL build-date=$BUILD_DATE
LABEL git-revision=$GIT_REVISION

EXPOSE 9600
ENV PORT 9600
