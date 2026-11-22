# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.90
ARG DEBIAN_RUNTIME_IMAGE=debian:trixie-slim
ARG DISTROLESS_IMAGE=gcr.io/distroless/cc-debian13:nonroot

# ==============================================================================
# Shared Rust toolchain + cargo-chef base
# ==============================================================================
FROM rust:${RUST_VERSION}-trixie AS chef

ENV RUSTUP_TOOLCHAIN=stable \
    CARGO_TERM_COLOR=always

RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler \
        pkg-config \
        libssl-dev \
        python3 \
    && rm -rf /var/lib/apt/lists/*

RUN --mount=type=cache,id=ferriscraft-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git,target=/usr/local/cargo/git \
    cargo install cargo-chef --version 0.1.77 --locked

RUN mkdir -p /distroless-app && chown 65532:65532 /distroless-app

# ==============================================================================
# Shared distroless runtime for pure-Rust plugin binaries
# ==============================================================================
FROM ${DISTROLESS_IMAGE} AS distroless-runtime

WORKDIR /app
COPY --from=chef --chown=65532:65532 /distroless-app /app

# ==============================================================================
# Pipeline 1: Orchestrator + Map-Sync (independent of rspigot build-context)
# ==============================================================================
FROM chef AS orchestrator-src

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ferriscraft-protocol ./crates/ferriscraft-protocol
COPY crates/ferriscraft-progression ./crates/ferriscraft-progression
COPY crates/ferriscraft-progression-pg ./crates/ferriscraft-progression-pg
COPY crates/ferriscraft-orchestrator ./crates/ferriscraft-orchestrator

# Build only orchestrator + protocol without requiring rspigot path deps
RUN python3 -c 'txt = open("Cargo.toml").read(); open("Cargo.toml", "w").write("\n".join(l for l in txt.splitlines() if "rspigot-" not in l) + "\n")' \
    && sed -i 's/members = \["crates\/\*"\]/members = ["crates\/ferriscraft-protocol", "crates\/ferriscraft-progression", "crates\/ferriscraft-progression-pg", "crates\/ferriscraft-orchestrator"]/' Cargo.toml

FROM orchestrator-src AS orchestrator-planner

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS orchestrator-builder

WORKDIR /app
COPY --from=orchestrator-planner /app/recipe.json recipe.json
RUN --mount=type=cache,id=ferriscraft-cargo-registry-orchestrator,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git-orchestrator,sharing=locked,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json

COPY --from=orchestrator-src /app/Cargo.toml /app/Cargo.lock ./
COPY crates/ferriscraft-protocol ./crates/ferriscraft-protocol
COPY crates/ferriscraft-progression ./crates/ferriscraft-progression
COPY crates/ferriscraft-progression-pg ./crates/ferriscraft-progression-pg
COPY crates/ferriscraft-orchestrator ./crates/ferriscraft-orchestrator
RUN --mount=type=cache,id=ferriscraft-cargo-registry-orchestrator,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git-orchestrator,sharing=locked,target=/usr/local/cargo/git \
    cargo build --release -p ferriscraft-orchestrator --bin ferriscraft-orchestrator --bin ferriscraft-map-sync \
    && cargo build --release -p ferriscraft-progression-pg --bin ferriscraft-migrate

FROM ${DEBIAN_RUNTIME_IMAGE} AS orchestrator

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        unzip \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /app ferriscraft

WORKDIR /app
COPY --from=orchestrator-builder /app/target/release/ferriscraft-orchestrator /usr/local/bin/ferriscraft-orchestrator
COPY --from=orchestrator-builder /app/target/release/ferriscraft-map-sync /usr/local/bin/ferriscraft-map-sync
COPY --from=orchestrator-builder /app/target/release/ferriscraft-migrate /usr/local/bin/ferriscraft-migrate

USER ferriscraft
EXPOSE 8080
ARG FERRISCRAFT_VERSION=dev
ENV FERRISCRAFT_VERSION=${FERRISCRAFT_VERSION}
ENTRYPOINT ["/usr/local/bin/ferriscraft-orchestrator"]

# ==============================================================================
# Pipeline 2: rSpigot Minecraft Server (requires --build-context rspigot-src=...)
# ==============================================================================
FROM chef AS rspigot-planner

WORKDIR /src/rspigot
COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS rspigot-cook

ARG CARGO_PROFILE_RELEASE_LTO=false
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
    CARGO_PROFILE_RELEASE_LTO=${CARGO_PROFILE_RELEASE_LTO}
WORKDIR /src/rspigot
COPY --from=rspigot-planner /src/rspigot/recipe.json recipe.json
RUN --mount=type=cache,id=ferriscraft-cargo-registry-rspigot,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git-rspigot,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=ferriscraft-cargo-target-rspigot,sharing=locked,target=/src/rspigot/target \
    cargo chef cook --release -p rspigot --recipe-path recipe.json

FROM rspigot-cook AS rspigot-deps-builder

COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . .
RUN printf 'fn main() {}\n' > apps/rspigot/src/main.rs
RUN --mount=type=cache,id=ferriscraft-cargo-registry-rspigot,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git-rspigot,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=ferriscraft-cargo-target-rspigot,sharing=locked,target=/src/rspigot/target \
    cargo build --release -p rspigot

FROM rspigot-deps-builder AS rspigot-builder

COPY --from=rspigot-src apps/rspigot ./apps/rspigot
RUN --mount=type=cache,id=ferriscraft-cargo-registry-rspigot,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git-rspigot,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=ferriscraft-cargo-target-rspigot,sharing=locked,target=/src/rspigot/target \
    touch apps/rspigot/src/main.rs \
    && cargo build --release -p rspigot \
    && cp /src/rspigot/target/release/rspigot /usr/local/bin/rspigot

FROM ${DEBIAN_RUNTIME_IMAGE} AS rspigot

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        python3 \
        unzip \
    && rm -rf /var/lib/apt/lists/*

COPY --from=rspigot-builder /usr/local/bin/rspigot /usr/local/bin/rspigot
COPY server/bukkit.yml server/permissions.yml server/server.properties server/spigot.yml.example server/world.zip /opt/ferriscraft/templates/lobby/
COPY server-sheepwars/bukkit.yml server-sheepwars/permissions.yml server-sheepwars/server.properties server-sheepwars/spigot.yml.example server-sheepwars/arena.jso[n] server-sheepwars/world.zi[p] /opt/ferriscraft/templates/sheepwars/
COPY --chmod=0755 deploy/docker/rspigot-entrypoint.sh /usr/local/bin/rspigot-entrypoint.sh

EXPOSE 25565 25580 25581
ARG FERRISCRAFT_VERSION=dev
ENV FERRISCRAFT_VERSION=${FERRISCRAFT_VERSION}
ENTRYPOINT ["/usr/local/bin/rspigot-entrypoint.sh"]

# ==============================================================================
# Pipeline 3: FerrisCraft Rust Plugins (lobby + sheepwars)
# ==============================================================================
FROM chef AS plugins-planner

COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . /opt/minecraft/dev/rspigot
WORKDIR /opt/ferrislabs/ferriscraft
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS plugins-cook

ARG CARGO_PROFILE_RELEASE_LTO=false
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
    CARGO_PROFILE_RELEASE_LTO=${CARGO_PROFILE_RELEASE_LTO}
COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . /opt/minecraft/dev/rspigot
WORKDIR /opt/ferrislabs/ferriscraft
COPY --from=plugins-planner /opt/ferrislabs/ferriscraft/recipe.json recipe.json
RUN --mount=type=cache,id=ferriscraft-cargo-registry-plugins,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git-plugins,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=ferriscraft-cargo-target-lobby,sharing=locked,target=/opt/ferrislabs/ferriscraft/target \
    cargo chef cook --release --bin ferriscraft-lobby --recipe-path recipe.json

FROM plugins-cook AS plugins-builder

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,id=ferriscraft-cargo-registry-plugins,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git-plugins,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=ferriscraft-cargo-target-lobby,sharing=locked,target=/opt/ferrislabs/ferriscraft/target \
    cargo build --release --bin ferriscraft-lobby \
    && cp /opt/ferrislabs/ferriscraft/target/release/ferriscraft-lobby /usr/local/bin/ferriscraft-lobby

FROM distroless-runtime AS lobby

COPY --from=plugins-builder /usr/local/bin/ferriscraft-lobby /usr/local/bin/ferriscraft-lobby
COPY --chown=65532:65532 lobby.json /app/lobby.json
ARG FERRISCRAFT_VERSION=dev
ENV FERRISCRAFT_VERSION=${FERRISCRAFT_VERSION}
ENTRYPOINT ["/usr/local/bin/ferriscraft-lobby"]

FROM chef AS sheepwars-cook

COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . /opt/minecraft/dev/rspigot
WORKDIR /opt/ferrislabs/ferriscraft
COPY --from=plugins-planner /opt/ferrislabs/ferriscraft/recipe.json recipe.json
RUN --mount=type=cache,id=ferriscraft-cargo-registry-plugins,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git-plugins,sharing=locked,target=/usr/local/cargo/git \
    cargo chef cook --release --bin ferriscraft-sheepwars --recipe-path recipe.json

FROM sheepwars-cook AS sheepwars-builder

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,id=ferriscraft-cargo-registry-plugins,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ferriscraft-cargo-git-plugins,sharing=locked,target=/usr/local/cargo/git \
    cargo build --release --bin ferriscraft-sheepwars

FROM distroless-runtime AS sheepwars

COPY --from=sheepwars-builder /opt/ferrislabs/ferriscraft/target/release/ferriscraft-sheepwars /usr/local/bin/ferriscraft-sheepwars
ARG FERRISCRAFT_VERSION=dev
ENV FERRISCRAFT_VERSION=${FERRISCRAFT_VERSION}
ENTRYPOINT ["/usr/local/bin/ferriscraft-sheepwars"]

# ==============================================================================
# Pipeline 4: Velocity Proxy + FerrisCraft Velocity Plugin
# ==============================================================================
FROM maven:3.9-eclipse-temurin-25 AS velocity-builder

WORKDIR /build
ARG VELOCITY_URL="https://fill-data.papermc.io/v1/objects/0844bcabdcdb4f5b875906fd4c3365bc1bb76918e42bc595ed88ad195258dd75/velocity-4.3.1-45.jar"
ARG VELOCITY_SHA256="0844bcabdcdb4f5b875906fd4c3365bc1bb76918e42bc595ed88ad195258dd75"

RUN curl -fSL -o /build/velocity.jar "${VELOCITY_URL}" \
    && echo "${VELOCITY_SHA256}  /build/velocity.jar" | sha256sum -c -

COPY proxy/velocity-plugin/pom.xml ./
RUN --mount=type=cache,id=ferriscraft-maven-repo,target=/root/.m2 \
    mvn -B dependency:go-offline

COPY proxy/velocity-plugin/src ./src
RUN --mount=type=cache,id=ferriscraft-maven-repo,target=/root/.m2 \
    mvn -B package

FROM eclipse-temurin:25-jre-noble AS velocity

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=velocity-builder /build/velocity.jar /app/velocity.jar
COPY --from=velocity-builder /build/target/ferriscraft-velocity.jar /app/plugins/ferriscraft-velocity.jar
COPY --chmod=0755 deploy/docker/velocity-entrypoint.sh /usr/local/bin/velocity-entrypoint.sh

EXPOSE 25565
ARG FERRISCRAFT_VERSION=dev
ENV FERRISCRAFT_VERSION=${FERRISCRAFT_VERSION}
ENTRYPOINT ["/usr/local/bin/velocity-entrypoint.sh"]
