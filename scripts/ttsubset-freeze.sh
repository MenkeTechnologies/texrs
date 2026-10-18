#!/usr/bin/env bash
# Record the TrueType subsets luatex embeds, for tests/ttsubset.rs.
#
# A page per font is set by luahbtex with luaotfload in mode=node and written
# with compression off, and the font's /FontFile2 stream and /CIDSet stream
# are cut out of the PDF into tests/data/ttsubset/FONT.fontfile and
# FONT.cidset. The test rebuilds the subset of the glyphs the CIDSet names
# from the same font file and requires the same bytes.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
dir="$repo/tests/data/ttsubset"
work=$(mktemp -d)
trap 'command rm -rf "$work"' EXIT
mkdir -p "$dir"

fonts=(DejaVuSans.ttf Arimo-Regular.ttf NotoSerif-Regular.ttf)
text='Office affine flow -- Ångström é ü ñ Ŵ ǅ ѐ Ἀθῆναι 0123456789 AVAT'

cat >"$work/used.lua" <<'LUA'
function dumpused(cs, path)
  local f = font.getcopy(font.id(cs))
  local out = io.open(path, "w")
  local ks = {} for k, c in pairs(f.characters) do if c.used then ks[#ks + 1] = k end end table.sort(ks)
  for _, k in ipairs(ks) do
    out:write(string.format("%X %d %s\n", k, f.characters[k].index, tostring(f.characters[k].tounicode)))
  end
  out:close()
end
LUA

for f in "${fonts[@]}"; do
  cat >"$work/run.tex" <<TEX
\\input luaotfload.sty
\\pdfvariable compresslevel=0
\\pdfvariable objcompresslevel=0
\\font\\x="file:$f:mode=node" at 10pt
\\x $text\\par\\vfill\\penalty-10000
\\directlua{dofile("used.lua") dumpused("x", "used.txt")}
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
    $pdf =~ m{/FontFile2 (\d+) 0 R} or die "no FontFile2"; my $ff = stream_of($1);
    $pdf =~ m{/CIDSet (\d+) 0 R} or die "no CIDSet"; my $cs = stream_of($1);
    $pdf =~ m{/ToUnicode (\d+) 0 R} or die "no ToUnicode"; my $tu = stream_of($1);
    $pdf =~ m{/FontDescriptor\s*/FontName /(\S+)|/FontName /([A-Z]{6}\+\S+)} or die "no FontName"; my $fn = $1 // $2;
    open my $t, ">:raw", $ARGV[3]; print $t $tu; close $t;
    open $t, ">:raw", $ARGV[4]; print $t "$fn\n"; close $t;
    open my $o, ">:raw", $ARGV[1]; print $o $ff; close $o;
    open $o, ">:raw", $ARGV[2]; print $o $cs; close $o;
  ' "$work/run.pdf" "$dir/${f%.ttf}.fontfile" "$dir/${f%.ttf}.cidset" "$dir/${f%.ttf}.tounicode" "$dir/${f%.ttf}.fontname"
  cp "$work/used.txt" "$dir/${f%.ttf}.used"
done
luahbtex --version | head -1 >"$dir/VERSION"
echo "wrote $dir"
