# dev_engine — assignment autograder.
#
# Same shape as IronJudge's image (`lamicons-code-engine/apps/judge/Dockerfile`): build the Rust
# worker, bake a sandbox rootfs from separate stages, and ship both in a slim runner.
#
# The rootfs holds the **toolchain only** — python+pip, bun, jdk+maven. It deliberately does *not*
# hold any third-party dependency: every project declares its own, so they are resolved per
# submission at grade time, by the sandbox's online install phase (see `contract.md` §4). That is
# why this image does not need to know anything about the assignments, and why changing an
# assignment's dependencies needs no rebuild.
#
# Build from the repo root:  docker build -f apps/grader/Dockerfile -t lamicons-dev-engine .

# ---------------------------------------------------------------- the worker binary
FROM rust:1-slim-bookworm AS builder
# librdkafka is built from source (configure+make, no cmake/openssl — see apps/grader/Cargo.toml).
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config zlib1g-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
RUN cargo build --release --bins

# ---------------------------------------------------------------- python rootfs
FROM debian:bookworm-slim AS rootfs-python
RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 python3-venv python3-pip ca-certificates \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

# ---------------------------------------------------------------- javascript (bun) rootfs
FROM debian:bookworm-slim AS rootfs-javascript
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl unzip libstdc++6 \
    && curl -fsSL https://bun.sh/install | bash \
    && mv /root/.bun/bin/bun /usr/local/bin/bun \
    && chmod 0755 /usr/local/bin/bun \
    && apt-get purge -y curl unzip && apt-get autoremove -y \
    && apt-get clean && rm -rf /var/lib/apt/lists/* /root/.bun

# ---------------------------------------------------------------- java (maven) rootfs
FROM debian:bookworm-slim AS rootfs-java
RUN apt-get update && apt-get install -y --no-install-recommends \
    openjdk-17-jdk-headless maven ca-certificates \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

# ---------------------------------------------------------------- runner
FROM debian:bookworm-slim AS runner
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

# One rootfs per stack, at ROOTFS_BASE/<stack> — the sandbox bind-mounts /usr /etc /opt /home (and
# the /bin, /sbin, /lib, /lib64 symlinks into them) from here, read-only, into every run.
COPY --from=rootfs-python     /usr /opt/sandbox_rootfs/python/usr
COPY --from=rootfs-python     /etc /opt/sandbox_rootfs/python/etc
COPY --from=rootfs-javascript /usr /opt/sandbox_rootfs/javascript/usr
COPY --from=rootfs-javascript /etc /opt/sandbox_rootfs/javascript/etc
COPY --from=rootfs-java       /usr /opt/sandbox_rootfs/java/usr
COPY --from=rootfs-java       /etc /opt/sandbox_rootfs/java/etc

# For each rootfs:
#   * Debian is merged-usr, so recreate the top-level symlinks absolute toolchain paths rely on;
#   * create /home/grader — the sandbox binds the submission's writable $HOME over it, which is
#     where pip, bun and maven put their caches;
#   * create empty /etc/resolv.conf and /etc/hosts — the online install phase bind-mounts the
#     host's real ones over these, and a bind needs its target to exist. Without them, DNS inside
#     the sandbox fails and no dependency can be downloaded.
RUN set -eux; \
    for stack in python javascript java; do \
        cd "/opt/sandbox_rootfs/$stack"; \
        ln -s usr/bin bin; \
        ln -s usr/sbin sbin; \
        ln -s usr/lib lib; \
        { [ -d usr/lib64 ] && ln -s usr/lib64 lib64; } || true; \
        mkdir -p home/grader; \
        touch etc/resolv.conf etc/hosts; \
    done

COPY --from=builder /app/target/release/grader      /usr/local/bin/grader
COPY --from=builder /app/target/release/grade-local /usr/local/bin/grade-local

# Assignments are mounted at runtime; the scratch dir is where per-submission student/work/home
# directories are assembled.
RUN mkdir -p /opt/assignments /tmp/grader
ENV ASSIGNMENTS_ROOT=/opt/assignments \
    ROOTFS_BASE=/opt/sandbox_rootfs \
    WORK_ROOT=/tmp/grader \
    RUST_LOG=info

WORKDIR /app
CMD ["grader"]
