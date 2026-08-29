#!/usr/bin/env perl
# gen.pl — generate random TeX programs for the differential fuzzer.
#
#   perl scripts/fuzz/gen.pl SEED N DIR [DEPTH]
#
# Writes N files DIR/f00000.tex .. and prints nothing. The same SEED produces
# byte-identical files on any machine: the PRNG is an explicit LCG rather than
# perl's `rand', whose sequence is not a portable contract.
#
# The grammar is deliberately CONFINED to what texrs implements (mouth,
# expander, count registers, conditionals, groups, \def/\edef/\let/\csname).
# Generating constructs from the stomach would produce divergences that are
# already written down in BUGS.md and would drown the ones worth reading.
#
# Three properties the grammar has to hold, each learned from a class of false
# divergence rather than from taste:
#
#   * No recursive macro. A body only calls macros defined BEFORE it, so every
#     program terminates under both engines. `\def\a{\a}` loops in real tex too,
#     and a hang is not a divergence -- both sides time out and compare equal.
#   * No arithmetic overflow. Counts stay under 10^4 and multipliers under 10, so
#     nothing approaches TeX's 2^31 register limit. An overflow is a real tex
#     error message, which would swamp the corpus with one uninteresting shape.
#   * No division by zero. `\divide` always gets a non-zero literal.
use strict;
use warnings;

my ($seed, $n, $dir, $depth) = @ARGV;
die "usage: gen.pl SEED N DIR [DEPTH]\n" unless defined $dir;
$depth //= 2;

my $state = ($seed + 1) & 0xFFFFFFFF;
# Numerical Recipes' 32-bit LCG. Small, and its low bits are never used.
sub rnd { my $m = shift; $state = ($state * 1664525 + 1013904223) & 0xFFFFFFFF; return ($state >> 16) % $m }
sub pick { my @a = @_; return $a[rnd(scalar @a)] }

# A piece that ENDS in a control word would swallow whatever letter follows it
# into its own name: `\fi' + `EPS' is the single control sequence `\fiEPS', which
# is undefined in both engines and produces an error rather than a comparison.
# One space fixes it and cannot change the output, because the mouth eats the
# space that terminates a control word.
sub cs_safe { my $s = shift; $s .= ' ' if $s =~ /\\[a-zA-Z]+$/; return $s }

my @WORDS = qw(ALPHA BETA GAMMA DELTA EPS ZETA ETA THETA IOTA KAPPA);

# The registers a generated program is allowed to touch.
#
# NOT \count0: the oracle is `tex' with the plain format preloaded, where
# \count0 is the page number and already holds 1 -- while texrs starts every
# register at zero, as INITEX does. Reading it compares two engines that were
# never in the same state, which is a divergence in the corpus rather than in
# the engine. Plain leaves \count1..\count9 at zero, and allocates from
# \count10 up, so 1..9 is the window where both engines agree at startup.
sub reg { return 1 + rnd(9) }

# The ZERO-ARGUMENT macros in scope where generation currently is.
#
# Arity matters: a macro defined as `\def\m#1{...}' called bare grabs whatever
# token happens to follow it, so a program that calls one is comparing two
# engines' argument grabbing rather than the construct it was generated for.
# Parameterised macros are therefore defined and called in the same breath, with
# a braced argument, and never enter this list.
#
# Scope matters for the same reason: a macro defined inside `{...}' is gone at
# the `}', so calling it later is an `Undefined control sequence' in real tex --
# a divergence about error reporting, in a program generated to test something
# else. The group branch saves and restores this list exactly as TeX's save
# stack does.
my @defined;

sub macro_name { my $i = shift; return "\\m" . chr(ord('a') + $i % 26) . chr(ord('a') + int($i / 26) % 26) }

# A piece of text safe inside \message{...}: no braces, no specials.
sub literal { return pick(@WORDS) }

# Something that expands to characters. $d bounds nesting.
sub expansion {
  my $d = shift;
  my @choices = (
    sub { literal() },
    # The trailing space is load-bearing: TeX's number scanner keeps reading
    # (and EXPANDING) after the digits until something that cannot be part of a
    # number stops it, so `\the\count6' followed by `\number478' scans as
    # register 6478 -- `Bad register code'. One space terminates the scan and is
    # then swallowed as the scan's optional terminator, so it prints nothing.
    sub { "\\the\\count" . reg() . " " },
    sub { "\\number" . rnd(1000) . " " },
    sub { "\\string\\" . pick(qw(foo bar undefinedcs)) },
  );
  push @choices, sub { pick(@defined) } if @defined;
  if ($d > 0) {
    push @choices, sub {
      my $r = reg();
      sprintf('\ifnum\count%d%s%d %s\else %s\fi', $r, pick('>', '<', '='), rnd(20), expansion($d - 1), expansion($d - 1));
    };
    push @choices, sub {
      sprintf('\ifodd\count%d %s\else %s\fi', reg(), expansion($d - 1), expansion($d - 1));
    };
    push @choices, sub {
      sprintf('\ifcase\count%d %s\or %s\or %s\else %s\fi', reg(), expansion($d - 1), expansion($d - 1), expansion($d - 1), expansion($d - 1));
    };
    push @choices, sub {
      sprintf('%s %s\else %s\fi', pick('\iftrue', '\iffalse'), expansion($d - 1), expansion($d - 1));
    };
    push @choices, sub { '\csname ' . pick(qw(alpha beta undefinedname)) . '\endcsname' };
    if (@defined >= 2) {
      push @choices, sub {
        sprintf('\ifx%s%s SAME\else DIFF\fi', pick(@defined), pick(@defined));
      };
    }
  }
  return cs_safe(pick(@choices)->());
}

# One statement. $d bounds group nesting.
sub statement {
  my ($d, $idx) = @_;
  my @choices = (
    sub { sprintf('\count%d=%d', reg(), rnd(2000) - 1000) },
    sub { sprintf('\advance\count%d by %d', reg(), rnd(200) - 100) },
    sub { sprintf('\multiply\count%d by %d', reg(), rnd(9) + 1) },
    sub { sprintf('\divide\count%d by %d', reg(), rnd(9) + 1) },
    sub { sprintf('\message{%s}', join('', map { expansion($depth) } 1 .. rnd(3) + 1)) },
    sub {
      my $m = macro_name($idx);
      my $body = expansion($depth);
      push @defined, $m;
      return "\\def$m\{$body\}";
    },
    sub {
      my $m = macro_name($idx);
      my $body = expansion($depth);
      push @defined, $m;
      # \edef freezes the expansion NOW; the point is that a later \count change
      # must not move it.
      return "\\edef$m\{$body\}";
    },
    sub {
      my $m = macro_name($idx);
      my $body = expansion($depth);
      return "\\def$m#1\{" . pick(@WORDS) . "-#1-$body\}\\message\{$m\{" . literal() . "\}\}";
    },
  );
  push @choices, sub {
    my $m = macro_name($idx + 100);
    my $src = pick(@defined);
    push @defined, $m;
    return sprintf('\let%s=%s', $m, $src);
  } if @defined;
  push @choices, sub {
    # A macro with an argument, called. Only defined-with-#1 macros take one, so
    # the call is emitted next to its own definition rather than guessed at.
    my $m = macro_name($idx);
    return "\\def$m#1\{[#1]\}\\message\{$m\{" . literal() . "\}\}";
  };
  if ($d > 0) {
    push @choices, sub {
      my @saved = @defined;
      my @inner = map { statement($d - 1, $idx * 10 + $_) } 1 .. rnd(2) + 1;
      # A group must restore whatever it changed; that is the whole point of it,
      # and the generator's own view of what is defined has to be restored with
      # it or the next statement calls something that no longer exists.
      @defined = @saved;
      return '{' . join(' ', @inner) . '}';
    };
  }
  return cs_safe(pick(@choices)->());
}

mkdir $dir unless -d $dir;
for my $i (0 .. $n - 1) {
  @defined = ();
  my @lines = ('\catcode`\{=1 \catcode`\}=2 \catcode`\#=6');
  push @lines, statement($depth, $_) for 1 .. rnd(6) + 3;
  # Always finish with a message, or the program measures nothing: the parity
  # contract for this milestone IS the message stream.
  push @lines, sprintf('\message{%s}', join('', map { expansion($depth) } 1 .. 2));
  push @lines, '\end';
  my $f = sprintf('%s/f%05d.tex', $dir, $i);
  open my $fh, '>', $f or die "$f: $!";
  print $fh join("\n", @lines), "\n";
  close $fh;
}
