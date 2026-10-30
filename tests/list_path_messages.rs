//! What a plain document's list path (`--dvi`) prints for the expansions
//! that ask about the list being built: the mode conditionals (§501), the
//! mode-bound quantities `\spacefactor` and `\prevdepth` (§418), and a
//! font's quantities in an `\edef` (§425, §578); and where a report made
//! before a page ships lands against the page's `[n]`.
//!
//! Pinned as tex 3.141592653 prints each document, line ends removed as
//! texrs's message stream removes them; when the pinned `tex` is on the PATH
//! each is also run through it and compared, DVI included.

mod common;

const PRINTED: &[(&str, &str)] = &[
    // §501 in each mode the lowerer follows: outer vertical, paragraph,
    // restricted horizontal (inner), and text math (inner).
    (
        "\\message{\\ifvmode V\\fi\\ifhmode H\\fi\\ifmmode M\\fi\\ifinner I\\fi}A\\message{\\ifvmode V\\fi\\ifhmode H\\fi\\ifinner I\\fi}\\hbox{\\message{\\ifhmode H\\fi\\ifinner I\\fi}}$\\message{\\ifmmode M\\fi\\ifinner I\\fi}$\\par\n\\end\n",
        "V H HI MI [1]",
    ),
    // In the output routine, internal vertical mode.
    (
        "\\write16{\\ifinner I\\fi\\ifvmode V\\fi\\ifhmode H\\fi}A\\par\n\\output={\\message{\\ifinner I\\fi\\ifvmode V\\fi}\\plainoutput}\\end\n",
        "IV [1]",
    ),
    // §1370 expands a `\write` with `mode:=0`, deferred or not: every mode
    // conditional is false there.
    (
        "\\write16{\\ifinner I\\fi\\ifvmode V\\fi\\ifhmode H\\fi}A\\par\n\\end\n",
        "[1]",
    ),
    (
        "\\immediate\\write16{[\\ifvmode V\\fi\\ifinner I\\fi]}A\\immediate\\write16{[\\ifhmode H\\fi]}\\par\\end\n",
        "[][][1]",
    ),
    // §418-§428: `\spacefactor` outside horizontal mode is `Improper`, and 0.
    (
        "\\message{\\the\\spacefactor}A\\message{\\the\\spacefactor}\\par\\end\n",
        "! Improper \\spacefactor.l.1 \\message{\\the\\spacefactor                             }A\\message{\\the\\spacefactor}\\par\\end0 999 [1] )(see the transcript file for additional information",
    ),
    // `\prevdepth` outside vertical mode too, and an integer 0 under `\the`;
    // in math mode neither belongs.
    (
        "\\message{[\\the\\prevdepth]}A\\message{[\\the\\prevdepth]}$\\message{[\\the\\spacefactor]}$\\par\\end\n",
        "[-1000.0pt]! Improper \\prevdepth.l.1 ...{[\\the\\prevdepth]}A\\message{[\\the\\prevdepth                                                  ]}$\\message{[\\the\\spacefac...[0]! Improper \\spacefactor.l.1 ...\\the\\prevdepth]}$\\message{[\\the\\spacefactor                                                  ]}$\\par\\end[0] [1] )(see the transcript file for additional information",
    ),
    // §425 and §578 in an `\edef`: `\fontdimen`, `\hyphenchar` and
    // `\skewchar` are the lowerer's font memory.
    (
        "\\edef\\x{\\the\\fontdimen6\\tenrm}\\message{\\x}\\edef\\y{\\the\\fontdimen2\\font}\\message{\\meaning\\y}A\\par\\end\n",
        "10.00002pt macro:->3.33333pt [1]",
    ),
    (
        "\\edef\\x{\\the\\hyphenchar\\tenrm/\\the\\skewchar\\teni/\\fontname\\tenrm}\\message{\\x}A\\par\\end\n",
        "45/127/cmr10 [1]",
    ),
    // §579's error for a parameter the font has not got, and a family font.
    (
        "\\edef\\x{\\the\\fontdimen99\\tenrm}\\message{[\\x]}\\edef\\y{\\the\\fontdimen5\\textfont1/\\the\\hyphenchar\\font}\\message{[\\y]}\\message{\\the\\fontdimen2\\tenrm}A\\par\\end\n",
        "! Font \\tenrm has only 7 fontdimen parameters.<recently read> \\tenrm                        l.1 \\edef\\x{\\the\\fontdimen99\\tenrm                                  }\\message{[\\x]}\\edef\\y{\\the\\fontdimen5\\tex...[0.0pt] [4.30554pt/45] 3.33333pt [1] )(see the transcript file for additional information",
    ),
    // §577's `Missing font identifier`, the token read again.
    (
        "\\edef\\x{\\the\\hyphenchar\\relax}\\message{[\\x]}A\\par\\end\n",
        "! Missing font identifier.<to be read again>                    \\relax l.1 \\edef\\x{\\the\\hyphenchar\\relax                                 }\\message{[\\x]}A\\par\\end[45\\relax ] [1] )(see the transcript file for additional information",
    ),
    // A report made before the page builder fires is printed before the
    // `[n]` of the page it ships, not after it -- and not inside the output
    // routine lowered there, which runs only where the builder fires it.
    (
        "\\foo\\hbox{}\n\\end\n",
        "! Undefined control sequence.l.1 \\foo        \\hbox{}[1] )(see the transcript file for additional information",
    ),
    (
        "x\\foo\\par\n\\end\n",
        "! Undefined control sequence.l.1 x\\foo         \\par[1] )(see the transcript file for additional information",
    ),
    (
        "\\count1=99999999999 x\\par\n\\end\n",
        "! Number too big.l.1 \\count1=9999999999                      9 x\\par[1.2147483647] )(see the transcript file for additional information",
    ),
    (
        "\\hbox{\\number\\relax}\n\\end\n",
        "! Missing number, treated as zero.<to be read again>                    \\relax l.1 \\hbox{\\number\\relax                       }[1] )(see the transcript file for additional information",
    ),
];

#[test]
fn the_list_path_prints_what_tex_prints() {
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
fn the_pinned_strings_and_the_dvi_agree_with_tex_when_it_is_installed() {
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
