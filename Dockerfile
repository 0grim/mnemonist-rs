# syntax=docker/dockerfile:1
#
# Port Mortem 2026 submission build.
#
# Five stages. `core` and `parity` are the verified ones, and they answer the
# two questions a reader of this port should ask: does the Rust crate stand on
# its own with no Node present, and does upstream's own unmodified test suite
# pass against it.
#
# Four commands are the hard requirement (.port-mortem.toml [verify]):
#   docker build -t port-mortem .
#   docker run --rm port-mortem
#   docker run --rm port-mortem ./tests/run.sh all
#   docker build -t pm-core --target core . && docker run --rm pm-core
#
# `tools` and `bench` build the benchmark harness described in
# docs/METHODOLOGY.md. They are not part of the four commands above and have
# not been build-verified in this image; the benchmark figures in the README
# were produced on a host, not in a container.

ARG RUST_VERSION=1.97.1
ARG NODE_VERSION=24.18.1

# ---------- builder ----------
FROM rust:${RUST_VERSION}-slim AS builder
WORKDIR /src
# Warm the dependency cache before copying real sources, so source-only edits
# don't invalidate the (slow) dependency-download layer.
#
# Every workspace member's manifest must be present, including bench/runner:
# Cargo refuses to resolve the workspace graph -- even for `cargo fetch` --
# with one missing, so a partial copy fails before any real work happens.
COPY Cargo.toml Cargo.lock ./
COPY crates/mnemonist-core/Cargo.toml crates/mnemonist-core/
COPY crates/mnemonist-napi/Cargo.toml crates/mnemonist-napi/
COPY crates/difffuzz/Cargo.toml       crates/difffuzz/
COPY bench/runner/Cargo.toml          bench/runner/
# difffuzz is a library crate (src/lib.rs) whose binary is auto-discovered at
# src/bin/difffuzz.rs, so no dummy main is needed: an absent auto-discovered
# target is not an error. An absent *explicit* one is, and bench-runner
# declares `[[bin]] path = "src/main.rs"`, so that file must exist for manifest
# resolution to succeed and gets a real dummy below.
RUN mkdir -p crates/mnemonist-core/src crates/mnemonist-napi/src crates/difffuzz/src bench/runner/src \
 && touch crates/mnemonist-core/src/lib.rs crates/mnemonist-napi/src/lib.rs crates/difffuzz/src/lib.rs \
 && echo 'fn main(){}' > bench/runner/src/main.rs \
 && cargo fetch
COPY . .
# Both binaries are built here: the `parity` stage needs mnemonist-napi and
# the `bench` stage needs bench-runner, and they share the dependency graph
# already fetched above, so building them together costs nothing extra.
RUN cargo build --release -p mnemonist-napi -p bench-runner

# ---------- core: NO NODE. mirrors the FFI-rule rebuttal in CI job 1 ----------
FROM rust:${RUST_VERSION}-slim AS core
WORKDIR /src
COPY --from=builder /src /src
RUN ! command -v node                       # fails the build if Node ever creeps in
CMD ["cargo", "test", "-p", "mnemonist-core", "--release"]

# ---------- tools: cached separately from the port build ----------
FROM rust:${RUST_VERSION}-slim AS tools
# Unpinned upstream: this is bench tooling only, not part of the four
# required verify commands, and its non-reproducibility is confined to that
# scope (noted, not hidden).
RUN cargo install hyperfine --root /out --locked

# ---------- bench: BOTH implementations in one container ----------
# Both sides of every comparison run in the same image, on the same CPU, so a
# figure is never a cross-machine subtraction.
#
# Two placement details that are easy to get wrong. RSS is measured by
# bench-runner's own `--baseline` flag (bench/runner/src/rss.rs), not by a
# separate binary. And `bench/node/run.js` resolves obliterator from
# ../../tests/.work/node_modules via NODE_PATH rather than from a
# bench/node_modules, so the harness package is installed into tests/.work
# here -- exactly where run.js already looks.
FROM node:${NODE_VERSION}-slim AS bench
WORKDIR /app
COPY tests/harness-package.json ./tests/.work/package.json
RUN cd tests/.work && npm install --no-audit --no-fund --silent
COPY --from=builder /src/target/release/bench-runner ./target/release/bench-runner
COPY --from=tools    /out/bin/hyperfine /usr/local/bin/hyperfine
COPY bench ./bench
# bench/run.sh takes one optional module argument and defaults to
# static-disjoint-set. Unlike tests/run.sh it has no "all" mode, so the CMD
# passes no argument; give `docker run` a module name to override.
CMD ["./bench/run.sh"]

# ---------- parity (default target -- MUST REMAIN LAST) ----------
FROM node:${NODE_VERSION}-slim AS parity
WORKDIR /app
ENV PM_NO_BUILD=1
# Pre-install harness deps at build time so `docker run` needs no network.
# This is safe to bake into a layer because tests/run.sh decides whether to
# install by checking for node_modules/mocha itself, not by comparing a
# package.json timestamp -- a copied file's mtime would otherwise retrigger
# the install on every run.
COPY tests/harness-package.json ./tests/.work/package.json
RUN cd tests/.work && npm install --no-audit --no-fund --silent
COPY --from=builder /src/target/release/libmnemonist_napi.so ./target/release/
COPY tests ./tests
# `tests/boundary/reentrancy.js` walks up from its own directory looking for
# `bench/upstream/sparse-set.js`, because it compares against the *real*
# upstream implementation rather than a description of it. Without this the
# image builds cleanly and then fails at run time with "cannot locate
# bench/upstream" — which is exactly how it was found: the Dockerfile was
# authored without a working Docker daemon, so this path could be traced but
# not executed. Static tracing caught six other COPY bugs and missed this one.
COPY bench/upstream ./bench/upstream
# README.md links into docs/ throughout, so docs/ is copied with it: a reader
# who explores the image should not hit dead links in the first file they open.
COPY README.md DECISIONS.md .port-mortem.toml ./
COPY LICENSE LICENSE-MNEMONIST LICENSE-OBLITERATOR NOTICE ./
COPY docs ./docs
CMD ["./tests/run.sh"]
