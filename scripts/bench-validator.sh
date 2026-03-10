#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-quick}"
BASELINE="${2:-main}"
BENCHES=("${@:3}")

if [[ ${#BENCHES[@]} -eq 0 ]]; then
  BENCHES=(string_validators combinators error_construction cache)
fi

case "$MODE" in
  quick|full|baseline|compare) ;;
  *)
    echo "Unsupported mode: $MODE"
    echo "Usage: ./scripts/bench-validator.sh [quick|full|baseline|compare] [baseline-name] [bench ...]"
    echo ""
    echo "Modes:"
    echo "  quick     PR profile — fast feedback, reduced samples (default)"
    echo "  full      Release profile — full statistical analysis"
    echo "  baseline  Save results as a named baseline"
    echo "  compare   Compare against a saved baseline"
    exit 1
    ;;
esac

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "Running nebula-validator benches"
echo "Mode: $MODE | Baseline: $BASELINE | Benches: ${BENCHES[*]}"

for bench in "${BENCHES[@]}"; do
  args=(bench -p nebula-validator --bench "$bench" --)

  case "$MODE" in
    quick)
      args+=(--quick)
      ;;
    full)
      # Uses criterion defaults (100 samples, 5s warmup)
      ;;
    baseline)
      args+=(--save-baseline "$BASELINE")
      ;;
    compare)
      args+=(--baseline "$BASELINE")
      ;;
  esac

  echo
  echo "==> cargo ${args[*]}"
  cargo "${args[@]}"
done

echo
echo "Done. Criterion outputs are in target/criterion/."
echo "HTML reports: target/criterion/<group>/report/index.html"
