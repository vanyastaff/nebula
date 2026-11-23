#!/usr/bin/env bash
# RUSTC_WORKSPACE_WRAPPER for the `external-types` CI job only.
#
# `cargo-check-external-types` 0.5.0 reads rustdoc JSON format 57, which only
# nightlies from before 1.98 emit, while the workspace targets Rust 1.100 and
# uses std APIs stabilised in 1.98-1.100. On that older nightly those APIs are
# still behind feature gates, so this wrapper enables them. Cargo applies a
# *workspace* wrapper to workspace members only, which matters: third-party
# and `no_std` crates would reject features they cannot see. Rustdoc on the
# checked crate gets the same list through RUSTDOCFLAGS in ci.yml.
#
# Delete this file (and its use in ci.yml) once a cargo-check-external-types
# release supports the rustdoc JSON format of a nightly >= 1.100.
set -euo pipefail

rustc="$1"
shift
exec "$rustc" "$@" -Aunused_features "-Zcrate-attr=feature(${NEBULA_NIGHTLY_FEATURES:?})"
