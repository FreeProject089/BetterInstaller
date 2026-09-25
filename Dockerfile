# BetterInstaller — Linux dev / CI image.
#
# Reproduces the GitHub Actions ubuntu job locally (same toolchain + system deps), so
# you can run the full gate before pushing — no need to fight cross-compilation on Windows.
#
#   docker build -t betterinstaller-dev .
#   # run the whole CI gate (fmt + clippy + test) against your working tree:
#   docker run --rm -e CARGO_TARGET_DIR=/tmp/t -v "${PWD}:/app" betterinstaller-dev
#   # or an arbitrary command:
#   docker run --rm -e CARGO_TARGET_DIR=/tmp/t -v "${PWD}:/app" betterinstaller-dev \
#       bash -c "cargo build --workspace --release"
#
# CARGO_TARGET_DIR=/tmp/t keeps Linux build artifacts OUT of your Windows ./target.

# Pinned by version AND digest, never `rust:latest`: a moving tag gave a different compiler
# (and a different Debian) every few weeks without a single commit here, so "green in Docker"
# said nothing about which toolchain was green. ci.yml installs `toolchain: stable`, so this
# pin is "stable on the day it was bumped": 1.98.1 is the stable release of 2026-09-01, and
# 1.98.1-trixie is the very image `rust:latest` pointed at on 2026-09-25 (same Debian 13 base,
# same system libraries as before the pin).
# To bump, change tag and digest TOGETHER, to the Rust version CI's `stable` currently gives
# (the "Install Rust" step of ci.yml prints it):
#   docker pull rust:<version>-trixie
#   docker inspect --format '{{index .RepoDigests 0}}' rust:<version>-trixie
FROM rust:1.98.1-trixie@sha256:a8a5f0a1e5fe7dfe1d352591e4a1c7dd2c08fd70475cae872cf3458ba0df0546

# Slint GUI backend (fontconfig + xcb) and rfd's GTK3 file-dialog backend.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libfontconfig-dev libxcb-shape0-dev libxcb-xfixes0-dev libgtk-3-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt

# The gate runs as an unprivileged user, not root. It compiles and runs code from the working
# tree (build scripts, proc-macros, tests): none of it needs root, and a container that runs
# it as root turns any mistake in it into a root-owned mess on the bind-mounted tree.
#
# UID/GID default to 1000, the usual first user on a Linux host, so files cargo writes into
# the bind mount (a Cargo.lock refresh, say) belong to you. A different host id:
#   docker build --build-arg UID="$(id -u)" --build-arg GID="$(id -g)" -t betterinstaller-dev .
# (Docker Desktop on Windows/macOS maps the bind mount's ownership itself; the default is fine.)
#
# What `dev` has to be able to write, and why each path is created HERE:
#   /app                   WORKDIR, the bind mount point.
#   /tmp/target            docker-compose.yml's CARGO_TARGET_DIR, a named volume (bi-target).
#   $CARGO_HOME            cargo keeps its package-cache lock and global-cache database at the
#                          top of CARGO_HOME, so the directory itself must be writable...
#   $CARGO_HOME/registry   ...and the crate cache below it (named volume bi-cargo-registry),
#   $CARGO_HOME/git        and the git-dependency cache (named volume bi-cargo-git).
# A NEW named volume takes the ownership of the image directory it is mounted over, so these
# must exist, owned by dev, in the image: left to Docker, the mount point is created root-owned
# and the first `cargo fetch` dies on "Permission denied".
# $CARGO_HOME/bin (the rustup proxies) and RUSTUP_HOME (the toolchain) stay as the rust image
# ships them: root-owned, but world-writable (it does `chmod -R a+w` on both so that any uid
# can use them). Nothing here relies on that: the components are installed above, as root,
# and nothing at run time installs into the toolchain. (A `chmod -R` to take the write bit
# away would copy the whole toolchain into a new layer, about a gigabyte, for a throwaway
# local container: not done.)
#
# Volumes created by an image from before this change are still root-owned (a volume is only
# initialised when it is empty). If `docker compose run --rm ci` says "Permission denied" under
# /usr/local/cargo or /tmp/target, drop them once: `docker compose down --volumes`.
ARG UID=1000
ARG GID=1000
RUN groupadd --non-unique --gid "${GID}" dev \
    && useradd --non-unique --uid "${UID}" --gid "${GID}" --create-home --shell /bin/bash dev \
    && mkdir -p /app /tmp/target "${CARGO_HOME}/registry" "${CARGO_HOME}/git" \
    && chown dev:dev /app /tmp/target "${CARGO_HOME}" \
    && chown -R dev:dev "${CARGO_HOME}/registry" "${CARGO_HOME}/git"
USER dev

WORKDIR /app

# No HEALTHCHECK, on purpose: this is a one-shot container that runs the gate and exits, there is
# no long-running service whose health could be probed. The Trivy finding for it (DS-0026) is a
# reviewed exclusion in .github/security/trivyignore.yaml, with an expiry.

# Default: the exact CI gate, in order.
# NOT a LOGIN shell. `bash -l` runs /etc/profile, which SETS PATH outright instead of
# appending to it — so the `/usr/local/cargo/bin` the rust image puts there is dropped, and
# every command below fails with `cargo: command not found`. The image builds, the container
# starts, and the gate this file exists to run never runs once.
CMD ["bash", "-c", "cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace --release"]
