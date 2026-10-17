ARG CODENAME=bookworm
FROM debian:${CODENAME}
ARG CODENAME
ARG RUST_VERSION=1.98.0
ARG UV_VERSION=0.11.13
ENV DEBIAN_FRONTEND=noninteractive UV_PYTHON_INSTALL_DIR=/opt/orion/python UV_PYTHON_PREFERENCE=only-managed
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl gnupg \
    build-essential git pkg-config cmake ninja-build libssl-dev libdbus-1-dev libusb-1.0-0-dev \
    libcap-dev libasound2-dev libffi-dev libjpeg-dev libdrm-dev libfmt-dev xz-utils \
    && curl -fsSL https://archive.raspberrypi.com/debian/pool/main/r/raspberrypi-archive-keyring/raspberrypi-archive-keyring_2025.1+rpt1_all.deb -o /tmp/keyring.deb \
    && echo '2e727149d7acb8cc7f604e66d0049161039c8aa1eaf1175e54f9e69d963d60e4  /tmp/keyring.deb' | sha256sum -c - \
    && dpkg -i /tmp/keyring.deb \
    && echo "deb [arch=arm64 signed-by=/usr/share/keyrings/raspberrypi-archive-keyring.gpg] https://archive.raspberrypi.com/debian/ ${CODENAME} main" > /etc/apt/sources.list.d/raspi.list \
    && apt-get update && apt-get install -y --no-install-recommends libcamera-dev libkms++-dev \
       libcamera-ipa libarchive-dev libgl1 libgomp1 \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://astral.sh/uv/${UV_VERSION}/install.sh | env UV_INSTALL_DIR=/usr/local/bin sh \
    && uv python install 3.12.13 \
    && uv venv --seed --python 3.12.13 /opt/builder
ENV PATH=/opt/builder/bin:/root/.cargo/bin:/usr/local/bin:/usr/bin:/bin
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION}
ENV CARGO_TARGET_DIR=/cache/target CARGO_HOME=/cache/cargo UV_CACHE_DIR=/cache/uv CARGO_BUILD_JOBS=2
WORKDIR /src
