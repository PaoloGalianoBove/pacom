FROM ubuntu:24.04 AS dev
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive

ARG USERNAME=ubuntu
ARG USER_UID=1000
ARG USER_GID=1000
ARG WORKSPACE_ROOT=/workspaces/docker-uprotocol

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        clang \
        build-essential \
        cmake \
        curl \
        findutils \
        g++ \
        git \
        iproute2 \
        iputils-ping \
        libboost-all-dev \
        libclang-dev \
        libssl-dev \
        net-tools \
        pkg-config \
        protobuf-compiler \
        sudo \
        vim \
        wget \
    && rm -rf /var/lib/apt/lists/*

RUN if id -u "${USERNAME}" > /dev/null 2>&1; then \
        usermod -u "${USER_UID}" "${USERNAME}" && \
        groupmod -g "${USER_GID}" "${USERNAME}"; \
    else \
        groupadd --gid "${USER_GID}" "${USERNAME}" && \
        useradd --uid "${USER_UID}" --gid "${USER_GID}" -m "${USERNAME}"; \
    fi && \
    mkdir -p "/home/${USERNAME}" && \
    chown -R "${USER_UID}:${USER_GID}" "/home/${USERNAME}" && \
    printf '%s ALL=(ALL) NOPASSWD:ALL\n' "${USERNAME}" > "/etc/sudoers.d/${USERNAME}" && \
    chmod 0440 "/etc/sudoers.d/${USERNAME}"

USER ${USERNAME}
WORKDIR /home/${USERNAME}

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y && \
    $HOME/.cargo/bin/rustup default stable

ENV PATH="/home/${USERNAME}/.cargo/bin:${PATH}"

ARG CPP_STD_VER=13
ENV GENERIC_CPP_STDLIB_PATH="/usr/include/c++/${CPP_STD_VER}"
ENV ARCH_SPECIFIC_CPP_STDLIB_PATH="/usr/include/x86_64-linux-gnu/c++/${CPP_STD_VER}"
ENV WORKSPACE_ROOT="${WORKSPACE_ROOT}"
ENV PACOM_DIR="${WORKSPACE_ROOT}/pacom"

WORKDIR ${WORKSPACE_ROOT}
COPY --chown=${USERNAME}:${USERNAME} ./pacom ${PACOM_DIR}
WORKDIR ${PACOM_DIR}

# Build pacom once in dev image to prime Cargo cache and compile native vsomeip dependency.
RUN cargo build --examples

USER root
# Make vsomeip runtime libs globally visible through the dynamic loader in dev containers.
RUN find "${PACOM_DIR}/target" -type f -name 'libvsomeip*.so*' -exec cp -a {} /usr/local/lib/ \; && \
    ldconfig

USER ${USERNAME}
WORKDIR ${PACOM_DIR}
EXPOSE 30491 30492 30490/udp
CMD ["/bin/bash"]

FROM dev AS builder
WORKDIR ${PACOM_DIR}
RUN cargo build --release --examples

USER root
RUN mkdir -p /opt/pacom-runtime/lib /opt/pacom-runtime/bin && \
    find "${PACOM_DIR}/target" -type f -name 'libvsomeip*.so*' -exec cp -a {} /opt/pacom-runtime/lib/ \; && \
    cp "${PACOM_DIR}/target/release/examples/server" /opt/pacom-runtime/bin/pacom-server && \
    cp "${PACOM_DIR}/target/release/examples/client" /opt/pacom-runtime/bin/pacom-client

FROM ubuntu:24.04 AS runtime
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        iproute2 \
        net-tools \
        iputils-ping \
        libboost-all-dev \
        libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /opt/pacom-runtime/lib/ /usr/local/lib/
COPY --from=builder /opt/pacom-runtime/bin/pacom-server /usr/local/bin/pacom-server
COPY --from=builder /opt/pacom-runtime/bin/pacom-client /usr/local/bin/pacom-client

RUN ldconfig

EXPOSE 30491 30492 30490/udp
CMD ["/bin/bash"]
