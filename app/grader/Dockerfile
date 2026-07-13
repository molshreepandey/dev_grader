# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Build the worker.
# ---------------------------------------------------------------------------
FROM rust:1.90-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake pkg-config libssl-dev zlib1g-dev libsasl2-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .
RUN cargo build --release --bin grader --bin grade_once

# ---------------------------------------------------------------------------
# Runtime. The image *is* the sandbox rootfs: the toolchain here is what the
# student's build sees bind-mounted read-only at /usr, /bin, /lib, /etc.
#
# Based on the Maven + Temurin 21 image because the assignment templates target
# Java 21, and building a JDK-21-capable Debian from parts is a losing game.
# ---------------------------------------------------------------------------
FROM maven:3.9-eclipse-temurin-21 AS runtime

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates git curl unzip \
        python3 python3-pip \
    && rm -rf /var/lib/apt/lists/*

# pytest has to be present system-wide: /usr is read-only inside the sandbox, so
# a student's `pip install --user` can only add to it, never bootstrap it.
RUN pip3 install --no-cache-dir --break-system-packages pytest

# bun runs the JavaScript suites. Its `--reporter=junit` is built in, so the
# JUnit report cannot be broken by a student editing package.json.
RUN curl -fsSL https://bun.sh/install | bash \
    && mv /root/.bun/bin/bun /usr/local/bin/bun \
    && chmod 755 /usr/local/bin/bun \
    && rm -rf /root/.bun

# Disk-backed work + cache root. Mount a volume here; never a tmpfs — an
# npm/m2/target tree is far too big to hold in RAM.
RUN mkdir -p /var/lib/grader/work /var/lib/grader/cache
VOLUME ["/var/lib/grader"]

ENV GRADER_WORK_DIR=/var/lib/grader/work \
    GRADER_CACHE_DIR=/var/lib/grader/cache \
    GRADER_ISOLATION=true \
    RUST_LOG=info

COPY --from=builder /build/target/release/grader /usr/local/bin/grader
COPY --from=builder /build/target/release/grade_once /usr/local/bin/grade_once

WORKDIR /var/lib/grader
CMD ["grader"]
