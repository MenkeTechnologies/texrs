#!/usr/bin/env bash
# Record the CFF subsets luatex embeds, for tests/cffsubset.rs.
#
# A page per OpenType CFF font is set by luahbtex with luaotfload in
# mode=node and written with compression off, and the font's /FontFile3
# stream, its /CIDSet stream and its /FontName are cut out of the PDF into
# tests/data/cffsubset/FONT.fontfile, FONT.cidset and FONT.fontname. The
# test rebuilds the subset of the glyphs the CIDSet names from the same font
# file and requires the same bytes.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
dir="$repo/tests/data/cffsubset"
work=$(mktemp -d)
trap 'command rm -rf "$work"' EXIT
mkdir -p "$dir"

# name-keyed (write_cff) and CID-keyed (write_cid_cff) fonts
fonts=(lmroman10-regular.otf texgyretermes-regular.otf STIXTwoMath-Regular.otf HaranoAjiMincho-Regular.otf)
latin='Office affine flow -- Ångström é ü ñ 0123456789 AVAT'
cjk='日本語の文章、漢字とかな。'

for f in "${fonts[@]}"; do
  case $f in
    HaranoAji*) text="$latin $cjk" ;;
    STIX*) text='+−×÷∑∫√∞≤≥ αβγ ABC' ;;
    *) text="$latin" ;;
  esac
  cat >"$work/run.tex" <<TEX
\\input luaotfload.sty
\\pdfvariable compresslevel=0
\\pdfvariable objcompresslevel=0
\\font\\x="file:$f:mode=node" at 10pt
\\x $text\\par\\vfill\\penalty-10000
\\bye
TEX
  (cd "$work" && luahbtex -interaction=nonstopmode -halt-on-error run.tex </dev/null >"$work/log" 2>&1) || {
    echo "luahbtex failed on $f:" >&2
    tail -20 "$work/log" >&2
    exit 1
  }
  perl -e '
    local $/; open my $fh, "<:raw", $ARGV[0] or die; my $pdf = <$fh>;
    sub stream_of { my ($n) = @_; $pdf =~ /(?:^|[\r\n])$n 0 obj\s*<<(.*?)>>\s*stream\r?\n/s or die "no object $n";
      my $dict = $1; my $start = $+[0]; $dict =~ m{/Length (\d+)} or die "no length"; return substr($pdf, $start, $1); }
    $pdf =~ m{/FontFile3 (\d+) 0 R} or die "no FontFile3"; my $ff = stream_of($1);
    $pdf =~ m{/CIDSet (\d+) 0 R} or die "no CIDSet"; my $cs = stream_of($1);
    $pdf =~ m{/FontName /([A-Z]{6}\+\S+?)[\s/>]} or die "no FontName"; my $fn = $1;
    open my $o, ">:raw", $ARGV[1]; print $o $ff; close $o;
    open $o, ">:raw", $ARGV[2]; print $o $cs; close $o;
    open $o, ">:raw", $ARGV[3]; print $o "$fn\n"; close $o;
  ' "$work/run.pdf" "$dir/${f%.otf}.fontfile" "$dir/${f%.otf}.cidset" "$dir/${f%.otf}.fontname"
done
luahbtex --version | head -1 >"$dir/VERSION"
echo "wrote $dir"
