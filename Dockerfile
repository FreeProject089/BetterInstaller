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
#       bash -lc "cargo build --workspace --release"
#
# CARGO_TARGET_DIR=/tmp/t keeps Linux build artifacts OUT of your Windows ./target.

FROM rust:latest

# Slint GUI backend (fontconfig + xcb) and rfd's GTK3 file-dialog backend.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libfontconfig-dev libxcb-shape0-dev libxcb-xfixes0-dev libgtk-3-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt

WORKDIR /app

# Default: the exact CI gate, in order.
CMD ["bash", "-lc", "cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace --release"]
