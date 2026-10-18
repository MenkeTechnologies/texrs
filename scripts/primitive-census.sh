#!/usr/bin/env bash
# primitive-census.sh -- every primitive luatex has, measured against texrs.
#
#   bash scripts/primitive-census.sh
#
# Asks luatex for its own primitive table (tex.primitives(), origins from
# tex.extraprimitives), asks both engines for \meaning of each, runs the
# cases in tests/luatex_cases through both, and writes the verdicts into the
# primitive-census section of BUGS.md and luatex's case output into
# tests/data/luatex_cases_expected.txt, and luatex's command code for each
# primitive into src/luatex_primitives.rs. Nothing in any of them is typed by hand:
# src/bin/primitive_census.rs says how each verdict is decided.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo run --quiet --bin primitive-census -- --table --freeze --bugs > /dev/null
