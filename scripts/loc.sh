#!/usr/bin/env bash
#
# Rust line counts per workspace crate, DERIVED rather than maintained.
#
# The workspace members are read from Cargo.toml, so a crate added later is
# counted without editing this script. Only `.rs` files are counted: the
# Markdown under docs/ is documentation, not port code, and is never included.
#
# Each crate's lines are split four ways, because a single total invites the
# reader to assume it is all implementation:
#
#   code    statements and declarations
#   doc     `///` and `//!` rustdoc lines
#   note    ordinary `//` comments
#   test    everything inside a `#[cfg(test)]` module
#
# `blank` is reported but excluded from `code`. The `test` split is by brace
# depth from the `#[cfg(test)]` attribute, which is exact for this workspace
# (no test module is opened inside a string or a macro body here).
#
# Usage:  scripts/loc.sh            table
#         scripts/loc.sh --totals   one line per crate, machine-readable
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

members=$(awk '/^\[workspace\]/{w=1} w&&/^members[[:space:]]*=/{m=1} m{print} m&&/\]/{exit}' Cargo.toml \
          | grep -oE '"[^"]+"' | tr -d '"')

count_crate() {
  find "$1" -name '*.rs' -not -path '*/target/*' -print0 2>/dev/null \
    | xargs -0 -r awk '
      FNR == 1 { intest = 0; depth = 0; pending = 0 }
      {
        total++
        line = $0
        sub(/^[ \t]+/, "", line)

        if (intest) {
          test++
          n = gsub(/\{/, "{", line) - gsub(/\}/, "}", line)
          depth += n
          if (depth <= 0) intest = 0
          next
        }
        if (line ~ /^#\[cfg\(test\)\]/) { pending = 1; test++; next }
        if (pending) {
          test++
          n = gsub(/\{/, "{", line) - gsub(/\}/, "}", line)
          if (n > 0) { intest = 1; depth = n; pending = 0 }
          next
        }
        if (line == "")            { blank++; next }
        if (line ~ /^(\/\/\/|\/\/!)/) { doc++;   next }
        if (line ~ /^\/\//)        { note++;  next }
        code++
      }
      END { printf "%d %d %d %d %d %d\n", total, code, doc, note, test, blank }
    '
}

if [ "${1:-}" = "--totals" ]; then
  for m in $members; do
    printf '%s %s\n' "$m" "$(count_crate "$m")"
  done
  exit 0
fi

printf '%-16s %8s %8s %8s %7s %8s %8s\n' crate total code doc note test blank
printf '%-16s %8s %8s %8s %7s %8s %8s\n' ---------------- -------- -------- -------- ------- -------- --------
T=0; C=0; D=0; N=0; S=0; B=0
for m in $members; do
  read -r total code doc note test blank <<<"$(count_crate "$m")"
  name=$(awk -F'"' '/^name[[:space:]]*=/{print $2; exit}' "$m/Cargo.toml" 2>/dev/null)
  printf '%-16s %8d %8d %8d %7d %8d %8d\n' "${name:-$(basename "$m")}" "$total" "$code" "$doc" "$note" "$test" "$blank"
  T=$((T+total)); C=$((C+code)); D=$((D+doc)); N=$((N+note)); S=$((S+test)); B=$((B+blank))
done
printf '%-16s %8s %8s %8s %7s %8s %8s\n' ---------------- -------- -------- -------- ------- -------- --------
printf '%-16s %8d %8d %8d %7d %8d %8d\n' TOTAL "$T" "$C" "$D" "$N" "$S" "$B"
