ARG BASE_IMAGE=ubuntu:24.04
FROM ${BASE_IMAGE}
ARG RUST_VERSION=1.98.0
COPY --from=ghcr.io/astral-sh/uv:0.11.13 /uv /uvx /usr/local/bin/
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:/usr/local/bin:/usr/bin:/bin
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git build-essential pkg-config libssl-dev libdbus-1-dev libcap-dev \
    python3 python3-dev python3-venv util-linux \
    && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh \
    && sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain ${RUST_VERSION} \
    && rm /tmp/rustup-init.sh
RUN uv venv --seed --python 3.11 /opt/wheels-311 \
    && uv venv --seed --python 3.12 /opt/wheels-312
ENV CARGO_HOME=/cache/cargo CARGO_TARGET_DIR=/cache/target UV_CACHE_DIR=/cache/uv PIP_CACHE_DIR=/cache/pip
ENV CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 MATURIN_PEP517_ARGS=--locked
WORKDIR /work
