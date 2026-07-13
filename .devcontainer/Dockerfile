FROM ubuntu:24.04
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive

ARG USERNAME=ubuntu
ARG USER_UID=1000
ARG USER_GID=1000

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        curl \
        g++ \
        git \
        libboost-all-dev \
        libclang-dev \
        libssl-dev \
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
    "$HOME/.cargo/bin/rustup" toolchain install stable && \
    "$HOME/.cargo/bin/rustup" default stable && \
    "$HOME/.cargo/bin/rustup" toolchain install nightly

ENV PATH="/home/${USERNAME}/.cargo/bin:${PATH}"
RUN echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> "$HOME/.bashrc"