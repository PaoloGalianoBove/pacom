FROM rust:1.89-bookworm AS builder
WORKDIR /app

RUN rustup component add rustfmt

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        cmake \
        ninja-build \
        pkg-config \
        clang \
        libclang-dev \
        libboost-system-dev \
        libboost-thread-dev \
        libboost-filesystem-dev \
        libboost-program-options-dev \
        libboost-log-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY examples ./examples

RUN CPP_VER="$(basename "$(ls -d /usr/include/c++/* | sort -V | tail -n1)")" \
    && export GENERIC_CPP_STDLIB_PATH="/usr/include/c++/${CPP_VER}" \
    && export ARCH_SPECIFIC_CPP_STDLIB_PATH="/usr/include/x86_64-linux-gnu/c++/${CPP_VER}" \
    && cargo build --release --examples

RUN mkdir -p /tmp/vsomeip-libs \
    && find /app/target/release/build -path '*/out/vsomeip/vsomeip-install/lib/libvsomeip*.so*' -exec cp -v {} /tmp/vsomeip-libs/ \; \
    && ls -l /tmp/vsomeip-libs

FROM debian:bookworm-slim
WORKDIR /opt/pacom

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libstdc++6 \
        libboost-system1.74.0 \
        libboost-thread1.74.0 \
        libboost-filesystem1.74.0 \
        libboost-program-options1.74.0 \
        libboost-log1.74.0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/examples/rtt_server /opt/pacom/bin/rtt_server
COPY --from=builder /app/target/release/examples/rtt_client /opt/pacom/bin/rtt_client
COPY --from=builder /app/target/release/examples/light_switch /opt/pacom/bin/light_switch
COPY --from=builder /app/target/release/examples/light_dashboard /opt/pacom/bin/light_dashboard
COPY --from=builder /app/target/release/examples/cloud_app /opt/pacom/bin/cloud_app
COPY --from=builder /app/examples /opt/pacom/examples
COPY --from=builder /tmp/vsomeip-libs/ /usr/local/lib/

RUN ldconfig

ENV LD_LIBRARY_PATH=/usr/local/lib

