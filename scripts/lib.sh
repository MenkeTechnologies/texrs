# lib.sh — the pieces scripts/parity.sh and scripts/fuzz_parity.sh share.
#
# Sourced, never executed. Everything here exists because the two harnesses MUST
# ask the oracle the same question: two extractions that differ by a space are
# two different parity contracts, and the one that is wrong reports divergences
# that are not there (or, worse, parity that is not there).

# The engine version every expectation in this tree was measured against, read
# from BUGS.md so the number cannot drift from the document it is written in.
pinned_tex_version() {
  perl -ne 'print $1 and last if /measured against \*\*tex ([0-9.]+)\*\*/' "$ROOT/BUGS.md"
}

# Resolve the reference engine and refuse to run against a different one.
#
# A mismatched oracle does not fail loudly: it reports a different set of
# divergences, which reads exactly like a regression in texrs. So the version is
# checked up front and printed, and TEX_VERSION_EXPECT is the deliberate
# override for a cross-version run.
oracle_tex() {
  local path version expect
  TEX="${TEX_ORACLE:-tex}"
  path=$(command -v "$TEX" 2>/dev/null) || { echo "no \`$TEX' on PATH — the harness has no oracle" >&2; return 2; }
  version=$("$TEX" --version 2>/dev/null | head -1 | perl -ne 'print $1 if /TeX ([0-9.]+)/')
  expect="${TEX_VERSION_EXPECT:-$(pinned_tex_version)}"
  if [ -z "$expect" ]; then
    echo "cannot determine the pinned oracle version: no \"measured against **tex X.Y**\" line in BUGS.md." >&2
    echo "Restore it, or set TEX_VERSION_EXPECT explicitly." >&2
    return 2
  fi
  if [ -z "$version" ]; then
    echo "\`$path --version' did not report a TeX version — refusing to trust it as ground truth." >&2
    return 2
  fi
  if [ "$version" != "$expect" ]; then
    echo "oracle is tex $version, but every expectation in this tree was measured against $expect." >&2
    echo "  resolved: $path   (from TEX_ORACLE=${TEX_ORACLE:-tex})" >&2
    echo "  Set TEX_VERSION_EXPECT=$version to accept this deliberately." >&2
    return 2
  fi
  ORACLE_PATH="$path" ORACLE_VERSION="$version"
  return 0
}

# The texrs binary under test.
subject_bin() {
  if [ -n "${TEXRS:-}" ]; then BIN="$TEXRS"
  elif [ -x "$ROOT/target/debug/texrs" ]; then BIN="$ROOT/target/debug/texrs"
  elif [ -x "$ROOT/target/release/texrs" ]; then BIN="$ROOT/target/release/texrs"
  else echo "no texrs binary — run \`cargo build' first" >&2; return 2; fi
}

# The `\message' stream out of `(./NAME.tex ... )'.
#
# tex breaks its terminal output at max_print_line (79 by default), and the break
# lands anywhere — including immediately after the filename, which leaves the
# messages on the NEXT line entirely. Reading one line therefore misreads any
# output over 79 columns as empty. The harness pins max_print_line high so tex
# does not wrap at all, and this still joins continuation lines so a run against
# a tex that ignores the setting degrades to a wrong answer rather than a silent
# empty one.
extract_messages() {
  perl -e '
    my $text = do { local $/; <STDIN> };
    return unless $text =~ /\(\.\/[^\s)]*\.tex/g;
    my $rest = substr($text, pos($text));
    # Up to the LAST close paren, not the first: a message can print one
    # itself, and the paren that closes the file is the last one tex writes
    # before the summary.
    $rest = $1 if $rest =~ /^(.*)\)/s;
    $rest =~ s/\n//g;          # unwrap: tex breaks mid-token, adding nothing
    $rest =~ s/^\s+|\s+$//g;
    print $rest;
  '
}

# Run a command under a wall-clock limit, in seconds.
#
# SIGALRM survives exec, so the alarm set here fires inside the exec'd engine.
# GNU `timeout' is not on a stock macOS, and a runaway macro expands forever in
# BOTH engines -- the fuzzer has to bound them or a single generated case stops
# the run.
run_to() { perl -e 'alarm shift; exec @ARGV or die' "$@"; }

# Seconds any one engine gets for one case.
: "${CASE_TIMEOUT:=10}"

# What real tex prints for $1, or the marker HANG.
reference() {
  local f="$1" d out rc
  d=$(mktemp -d)
  cp "$f" "$d/case.tex"
  # max_print_line: see extract_messages. Kpathsea reads it from the environment.
  out=$( cd "$d" && max_print_line=8000 run_to "$CASE_TIMEOUT" "$TEX" -interaction=nonstopmode case.tex 2>&1 )
  rc=$?
  if [ "$rc" -eq 142 ] || [ "$rc" -eq 14 ]; then command rm -rf "$d"; printf 'HANG'; return; fi
  command rm -rf "$d"
  printf '%s\n' "$out" | extract_messages
}

# What texrs prints for $1, or the marker ERROR / HANG.
subject() {
  local out rc
  out=$(run_to "$CASE_TIMEOUT" "$BIN" "$1" 2>&1); rc=$?
  if [ "$rc" -eq 142 ] || [ "$rc" -eq 14 ]; then printf 'HANG'; return; fi
  [ "$rc" -eq 0 ] || { printf 'ERROR'; return; }
  printf '%s\n' "$out" | extract_messages
}
