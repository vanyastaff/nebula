#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-single}"
BASELINE="${2:-main}"
BENCHES=("${@:3}")

if [[ ${#BENCHES[@]} -eq 0 ]]; then
  BENCHES=(manager rate_limiter circuit_breaker)
fi

if [[ "$MODE" != "single" && "$MODE" != "baseline" && "$MODE" != "compare" ]]; then
  echo "Unsupported mode: $MODE"
  echo "Usage: ./scripts/bench-resilience.sh [single|baseline|compare] [baseline-name] [bench ...]"
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "Running nebula-resilience benches"
echo "Mode: $MODE | Baseline: $BASELINE | Benches: ${BENCHES[*]}"

for bench in "${BENCHES[@]}"; do
  args=(bench -p nebula-resilience --bench "$bench" --)

  case "$MODE" in
    baseline)
      args+=(--save-baseline "$BASELINE")
      ;;
    compare)
      args+=(--baseline "$BASELINE")
      ;;
    single)
      ;;
  esac

  echo
  echo "==> cargo ${args[*]}"
  cargo "${args[@]}"
done

echo
echo "Done. Criterion outputs are in target/criterion/."
