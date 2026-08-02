# syntax=docker/dockerfile:1
#
# Port Mortem 2026 submission build. planning/DESIGN.md 12c specifies this in
# outline; several details below were corrected against the real repo rather
# than copied verbatim from that outline, because the outline predates parts
# of the implementation. Each correction is called out where it happens.
#
# Four commands are the hard requirement (.port-mortem.toml [verify]):
#   docker build -t port-mortem .
#   docker run --rm port-mortem
#   docker run --rm port-mortem ./tests/run.sh all
#   docker build -t pm-core --target core . && docker run --rm pm-core
#
# `tools` and `bench` exist for parity with the benchmark methodology
# (DESIGN.md 5.2, 12c.2) but are not part of the four required commands above
# and were not build-verified here (see the agent's report for why).

ARG RUST_VERSION=1.97.1
ARG NODE_VERSION=24.18.1

# ---------- builder ----------
FROM rust:${RUST_VERSION}-slim AS builder
WORKDIR /src
# Warm the dependency cache before copying real sources, so source-only edits
# don't invalidate the (slow) dependency-download layer.
#
# Correction vs DESIGN.md 12c: the workspace has a FOURTH member,
# bench/runner (added after that section was written), and Cargo refuses to
# resolve the workspace graph -- even for `cargo fetch` -- unless every
# member's manifest is present. Its manifest and dummy source are included
# here too, or `cargo fetch` fails before any real work happens.
COPY Cargo.toml Cargo.lock ./
COPY crates/mnemonist-core/Cargo.toml crates/mnemonist-core/
COPY crates/mnemonist-napi/Cargo.toml crates/mnemonist-napi/
COPY crates/difffuzz/Cargo.toml       crates/difffuzz/
COPY bench/runner/Cargo.toml          bench/runner/
# difffuzz is a library crate (src/lib.rs) with a separate auto-discovered
# binary under src/bin/difffuzz.rs -- DESIGN.md 12c's sketch touched a
# src/main.rs that the real crate does not have and does not need; an absent
# auto-discovered target is not an error, only an absent *explicit* one is
# (bench-runner declares `[[bin]] path = "src/main.rs"` explicitly, so that
# one must exist for manifest resolution to succeed and gets a real dummy).
RUN mkdir -p crates/mnemonist-core/src crates/mnemonist-napi/src crates/difffuzz/src bench/runner/src \
 && touch crates/mnemonist-core/src/lib.rs crates/mnemonist-napi/src/lib.rs crates/difffuzz/src/lib.rs \
 && echo 'fn main(){}' > bench/runner/src/main.rs \
 && cargo fetch
COPY . .
# Correction vs DESIGN.md 12c: that sketch builds only mnemonist-napi, but the
# `bench` stage below needs the bench-runner binary too, and nothing else
# in the sketch ever builds it. Built together since both share the
# already-fetched dependency graph.
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

# ---------- bench: BOTH implementations, one container (DESIGN.md 5.2) ----------
# Correction vs DESIGN.md 12c: that sketch assumes a bench/package.json that
# does not exist and a target/release/rss-baseline binary that was never
# built -- RSS is measured via bench-runner's own `--baseline` flag instead
# (bench/runner/src/rss.rs), there is no separate binary for it. It also
# assumes obliterator is installed under bench/node_modules, but
# bench/node/run.js actually resolves it from ../../tests/.work/node_modules
# via NODE_PATH (see the comment in that file) -- there never was a
# bench/package.json to install from. This stage installs the harness
# package into tests/.work instead, exactly where run.js already looks.
FROM node:${NODE_VERSION}-slim AS bench
WORKDIR /app
COPY tests/harness-package.json ./tests/.work/package.json
RUN cd tests/.work && npm install --no-audit --no-fund --silent
COPY --from=builder /src/target/release/bench-runner ./target/release/bench-runner
COPY --from=tools    /out/bin/hyperfine /usr/local/bin/hyperfine
COPY bench ./bench
# bench/run.sh takes one optional module argument (default
# static-disjoint-set) -- it has no "all" mode, unlike tests/run.sh, so
# DESIGN.md 12c's `CMD ["./bench/run.sh", "all"]` would fail. Left with no
# argument; pass a module name to `docker run` to override.
CMD ["./bench/run.sh"]

# ---------- parity (default target -- MUST REMAIN LAST) ----------
FROM node:${NODE_VERSION}-slim AS parity
WORKDIR /app
ENV PM_NO_BUILD=1
# Pre-install harness deps at build time so `docker run` needs no network.
# The content-comparison amendment DESIGN.md 12c describes for the npm-install
# trigger was not needed: the real tests/run.sh checks for the presence of
# node_modules/mocha, not a package.json mtime, so it is already
# Docker-safe (see tests/run.sh's own history for why the check is shaped
# this way).
COPY tests/harness-package.json ./tests/.work/package.json
RUN cd tests/.work && npm install --no-audit --no-fund --silent
COPY --from=builder /src/target/release/libmnemonist_napi.so ./target/release/
COPY tests ./tests
# Correction vs DESIGN.md 12c: that sketch also copies README.md and
# DECISIONS.md into the image. Neither file exists in this repo yet (they
# are named as a future, separately-regenerated deliverable in this task's
# brief), so copying them verbatim would break the build; omitted rather
# than fabricated.
COPY .port-mortem.toml ./
CMD ["./tests/run.sh"]
