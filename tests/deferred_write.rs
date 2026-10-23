//! `\openout`, `\write` and `\closeout` without `\immediate` on a plain
//! document's DVI path: §1341's whatsit nodes, carried out by §1373's
//! `out_what` when the page holding them is shipped (§1366-§1374).
//!
//! What each document prints is pinned as tex 3.141592653 prints it, with
//! the terminal's line ends removed as texrs's message stream removes them:
//! a page's `\write` falls between its `[1` and its `]`. When `tex` is on the
//! PATH the same documents also run through it, terminal, DVI and written
//! files compared, so a pinned string that drifted from tex shows up here.
//! Without `plain.tex` and `cmr10.tfm` the list path cannot run and the tests
//! say so and stop.

mod common;

use std::path::Path;
use std::process::Command;

/// Documents and what tex prints for each between `(./case.tex` and `)`.
const PRINTED: &[(&str, &str)] = &[
    // §1370 at the shipout: `\the\pageno` on each page's own number, and the
    // `\immediate` one at once, before the page.
    (
        "\\write16{hello \\the\\pageno}\\immediate\\write16{now}Text.\\vfill\\eject\
         \\write16{second \\the\\count0}More.\\end\n",
        "now[1hello 1] [2second 2]",
    ),
    // A register read, and a conditional on it decided, when the page ships:
    // `\count1` is 7 by then, not the 5 it was at the `\write`.
    (
        "\\count1=5 \\write16{c=\\the\\count1;\\ifnum\\count1>6 big\\else small\\fi}\\count1=7\n\
         Text.\\vfill\\eject\n\\end\n",
        "[1.7c=7;big]",
    ),
    // Inside a box `\shipout` ships directly.
    ("\\shipout\\hbox{\\write16{in box}x}\\end\n", "[1in box]"),
    // A box never shipped writes nothing; a copy writes as often as it ships.
    (
        "\\setbox1\\hbox{\\write16{never}}\\setbox2\\hbox{\\write16{twice}}\
         \\shipout\\copy2\\shipout\\box2 \\end\n",
        "[1twice] [1twice]",
    ),
    // §1374 does nothing while `doing_leaders`.
    (
        "\\shipout\\hbox to 30pt{\\leaders\\hbox to 10pt{\\write16{L}x\\hss}\\hfil\
         \\write16{after}}\\end\n",
        "[1after]",
    ),
    // A negative stream is the log's alone; the rest in page order.
    (
        "\\write-1{logonly}\\write16{one}\\write16{two}Hello.\\par\\end\n",
        "[1onetwo]",
    ),
    // Page order is the order `vlist_out` meets them: the lines of the
    // paragraph, then plain's footnote below `\box255`.
    (
        "\\hsize=100pt \\parindent=0pt\n\
         aaa bbb ccc ddd \\write16{w1} eee fff ggg hhh iii jjj kkk lll\\write16{w2} mmm nnn ooo ppp qqq.\\par\n\
         \\footnote{*}{foot\\write16{inner}}\\write16{w3}\n\\end\n",
        "[1w1w2w3inner]",
    ),
    // A box held in a register writes on the page it is put on.
    (
        "\\setbox1\\vbox{\\write16{held}}\\write16{page one}A.\\vfill\\eject\\box1 B.\\end\n",
        "[1page one] [2held]",
    ),
    // A stream that is not open, and one past the last, go to the terminal.
    (
        "\\write5{closed five}\\write200{past the last}A.\\end\n",
        "[1closed fivepast the last]",
    ),
    // `\message` around a deferred write.
    (
        "\\message{before}\\write16{w}\\message{mid}Hi.\\vfill\\eject\\message{after}\\end\n",
        "before mid [1w] after",
    ),
    // In a display, and moved out of a paragraph by `\vadjust`.
    ("$$a\\write16{math}$$\\vadjust{\\write16{vadj}}\\end\n", "[1mathvadj]"),
    // §1356's display: the stream, `*` past 15 and `-` below 0, the text
    // unexpanded with its `#` doubled, the file name as it was read.
    (
        "\\tracingonline1 \\showboxdepth9 \\showboxbreadth99\n\
         \\setbox1\\hbox{\\write3{a\\relax b #}\\openout2=foo.bar\\closeout2\\write-5{x}\\write20{y}}\\showbox1\n\
         \\end\n",
        "> \\box1=\\hbox(0.0+0.0)x0.0.\\write3{a\\relax b ##}.\\openout2=foo.bar.\\closeout2\
         .\\write-{x}.\\write*{y}! OK.l.2 ...r\\closeout2\\write-5{x}\\write20{y}}\\showbox1\
         \x20                                                  )(see the transcript file for \
         additional information",
    ),
    // Under `\tracingoutput` §638 prints `]` with the box display, and the
    // write follows it.
    (
        "\\tracingoutput1 \\tracingonline1 \\showboxdepth9 \\showboxbreadth99\n\
         \\shipout\\hbox{\\write16{traced}}\\end\n",
        "Completed box being shipped out [1]\\hbox(0.0+0.0)x0.0.\\write*{traced}traced",
    ),
];

#[test]
fn a_deferred_write_prints_when_its_page_ships() {
    if !common::list_path_runs() {
        return;
    }
    let mut differ = Vec::new();
    for (src, printed) in PRINTED {
        let (said, _) = texrs::run_tex_dvi(src).expect("the document runs");
        if said != *printed {
            differ.push(format!("{src:?}\n  tex:   {printed:?}\n  texrs: {said:?}"));
        }
    }
    assert!(
        differ.is_empty(),
        "printed otherwise than tex prints:\n{}",
        differ.join("\n")
    );
}

#[test]
fn the_pinned_terminal_and_the_dvi_agree_with_tex_when_it_is_installed() {
    if !common::list_path_runs() {
        return;
    }
    let Some(tex) = common::tex() else {
        return;
    };
    let mut differ = Vec::new();
    for (src, printed) in PRINTED {
        let dir = tempfile::tempdir().expect("tempdir");
        let theirs = common::tex_in(&tex, dir.path(), src);
        if theirs != *printed {
            differ.push(format!("{src:?}: tex now prints {theirs:?}"));
        }
        let (_, ours) = texrs::run_tex_dvi(src).expect("the document runs");
        let reference = std::fs::read(dir.path().join("case.dvi")).ok();
        if reference.as_deref().map(common::without_the_date)
            != ours.as_deref().map(common::without_the_date)
        {
            differ.push(format!("{src:?}: the DVI differs from tex's"));
        }
    }
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

/// A deferred `\openout`, `\write` and `\closeout` act in page order at each
/// shipout: the `\label` idiom writes each page's `\folio` (roman on a
/// negative `\pageno`), a write after the deferred `\closeout` goes to the
/// terminal, and an `\immediate` write lands before the deferred ones of the
/// page it is read on.
const FILES_DOCUMENT: &str = r"\newwrite\aux
\immediate\openout\aux=labels.aux
\def\label#1{\write\aux{\string\def\string#1{\folio}}}
\openout5=defer.txt
\write5{first \the\pageno}
One.\label\a
\vfill\eject
\pageno=-3
Two.\label\b \write5{second \folio}\closeout5
\write5{after close}
\immediate\write\aux{immediate}
\vfill\eject
\end
";

/// What tex 3.141592653 wrote, file by file.
const FILES_WRITTEN: &[(&str, &str)] = &[
    ("labels.aux", "\\def\\a{1}\nimmediate\n\\def\\b{iii}\n"),
    ("defer.txt", "first 1\nsecond iii\n"),
];

fn texrs_dvi(dir: &Path) {
    std::fs::write(dir.join("case.tex"), FILES_DOCUMENT).expect("write the document");
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .args(["--dvi", "case.tex"])
        .current_dir(dir)
        .output()
        .expect("run texrs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn deferred_file_whatsits_write_the_files_tex_writes() {
    if !common::list_path_runs() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    texrs_dvi(dir.path());
    for (name, bytes) in FILES_WRITTEN {
        let got = std::fs::read_to_string(dir.path().join(name))
            .unwrap_or_else(|_| panic!("{name} was not written"));
        assert_eq!(&got, bytes, "{name}");
    }
    let Some(tex) = common::tex() else {
        return;
    };
    let theirs = tempfile::tempdir().expect("tempdir");
    let _ = common::tex_in(&tex, theirs.path(), FILES_DOCUMENT);
    for (name, _) in FILES_WRITTEN {
        let a = std::fs::read(theirs.path().join(name)).expect("tex wrote it");
        let b = std::fs::read(dir.path().join(name)).expect("texrs wrote it");
        assert_eq!(a, b, "{name}");
    }
    let a = std::fs::read(theirs.path().join("case.dvi")).expect("tex wrote a DVI");
    let b = std::fs::read(dir.path().join("case.dvi")).expect("texrs wrote a DVI");
    assert_eq!(
        common::without_the_date(&a),
        common::without_the_date(&b),
        "case.dvi"
    );
}
