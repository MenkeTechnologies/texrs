//! `\special` parsing against what tex actually writes, and dimensions against
//! what tex actually computes.
//!
//! A special is the one part of a DVI file TeX does not interpret, so the test
//! is the round trip: put specials in a document, let real tex write it, read
//! them back out of the DVI with the DVI reader, and parse them. Anything lost
//! between the `\special` and the parsed value is lost for a driver too.
//!
//! The dimensions get a better oracle still. A dimension assigned to a `\count`
//! prints in scaled points exactly, so tex can be asked what any dimension is
//! worth and the answer compared with this -- which is how the arithmetic here
//! came to be TeX's rather than the one that looks right.

use std::path::PathBuf;
use std::process::Command;

use texrs::special::{parse, Colour, Special};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_special_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run real tex over `source` and hand back the DVI, or `None` when there is
/// no tex here.
fn typeset(dir: &std::path::Path, source: &str) -> Option<Vec<u8>> {
    std::fs::write(dir.join("t.tex"), source).ok()?;
    let ran = Command::new("tex")
        .arg("-interaction=batchmode")
        .arg("t.tex")
        .current_dir(dir)
        .output()
        .ok()?;
    let _ = ran;
    std::fs::read(dir.join("t.dvi")).ok()
}

/// Everything the DVI carries as a special.
fn specials(dvi: &[u8]) -> Vec<String> {
    texrs::dvi::Dvi::parse(dvi)
        .expect("the dvi parses")
        .ops
        .iter()
        .filter_map(|op| match op {
            texrs::dvi::Op::Special(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The specials a document really carries, through tex and back.
#[test]
fn what_tex_wrote_is_what_this_reads() {
    let dir = scratch("round");
    let source = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\
         \\special{color push rgb 1 0 0}\n\
         \\special{papersize=210mm,297mm}\n\
         \\special{PSfile=\"fig.eps\" llx=0 lly=0 urx=100 ury=50 rwi=1000}\n\
         \\special{pdf:dest (section.1) [ @thispage /XYZ @xpos @ypos null ]}\n\
         \\special{html:<a href=\"http://example.com\">}\n\
         \\special{color pop}\n\
         Hello\n\\bye\n";
    let Some(dvi) = typeset(&dir, source) else {
        return;
    };
    let carried = specials(&dvi);
    assert_eq!(carried.len(), 6, "{carried:?}");

    let read: Vec<Special> = carried.iter().map(|text| parse(text)).collect();
    assert_eq!(read[0], Special::ColourPush(Colour::Rgb(1.0, 0.0, 0.0)));
    assert_eq!(
        read[1],
        Special::PaperSize {
            width: 39158276,
            height: 55380990
        },
        "A4, in the scaled points tex would have used"
    );
    match &read[2] {
        Special::Figure {
            name, bbox, width, ..
        } => {
            assert_eq!(name, "fig.eps");
            assert_eq!(bbox, &[0.0, 0.0, 100.0, 50.0]);
            assert_eq!(*width, Some(100.0));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&read[3], Special::PdfDest { name, .. } if name == "section.1"));
    assert_eq!(
        read[4],
        Special::HtmlAnchor {
            href: "http://example.com".into()
        }
    );
    assert_eq!(read[5], Special::ColourPop);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A special tex wrote with characters that need care: braces are catcoded
/// away, and everything else goes through as it was.
#[test]
fn a_special_reaches_the_driver_as_it_was_written() {
    let dir = scratch("verbatim");
    let source = "\\catcode`\\{=1 \\catcode`\\}=2\n\
         \\special{ps: gsave 0.5 setgray 0 0 moveto stroke grestore}\n\
         \\special{color push cmyk 0 1 1 0}\n\
         \\special{background gray 0.9}\n\
         Hi\n\\bye\n";
    let Some(dvi) = typeset(&dir, source) else {
        return;
    };
    let carried = specials(&dvi);

    // Unknown families come back whole rather than mangled or dropped.
    assert_eq!(
        parse(&carried[0]),
        Special::Unknown("ps: gsave 0.5 setgray 0 0 moveto stroke grestore".into())
    );
    assert_eq!(
        parse(&carried[1]),
        Special::ColourPush(Colour::Cmyk(0.0, 1.0, 1.0, 0.0))
    );
    // CMYK red is RGB red, which is what a page is drawn in.
    assert_eq!(Colour::Cmyk(0.0, 1.0, 1.0, 0.0).rgb(), (1.0, 0.0, 0.0));
    assert_eq!(parse(&carried[2]), Special::Background(Colour::Gray(0.9)));

    let _ = std::fs::remove_dir_all(&dir);
}

/// Every dimension, against tex's own arithmetic.
///
/// tex is asked for each one in scaled points by assigning it to a `\count`.
/// This is the test that decided how the parser works: converting to points
/// and scaling at the end agrees with tex on `1in` and disagrees on `1cc` and
/// on `210mm`.
#[test]
fn every_dimension_is_the_one_tex_computes() {
    let dir = scratch("dimen");
    let units = ["pt", "in", "pc", "cm", "mm", "bp", "dd", "cc", "sp"];
    let values = [
        "1", "0.5", "2.5", "10", "72", "210", "297", "0.001", "123.456",
    ];
    let written: Vec<String> = units
        .iter()
        .flat_map(|unit| values.iter().map(move |value| format!("{value}{unit}")))
        .collect();

    let mut source = String::from(
        "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\
         \\def\\sp#1{\\dimen0=#1 \\count0=\\dimen0 \\message{[\\the\\count0]}}\n",
    );
    for dimension in &written {
        source.push_str(&format!("\\sp{{{dimension}}}\n"));
    }
    source.push_str("\\end\n");

    if typeset(&dir, &source).is_none() && !dir.join("t.log").exists() {
        return;
    }
    let Ok(log) = std::fs::read_to_string(dir.join("t.log")) else {
        return;
    };
    // tex hard-wraps its log at 79 columns, and will break a number in half
    // to do it. Joining the lines back restores the stream, because the wrap
    // inserts a newline and nothing else.
    let log: String = log.lines().collect();
    let told: Vec<i64> = log
        .split("[")
        .skip(1)
        .filter_map(|piece| piece.split(']').next())
        .filter_map(|value| value.trim().parse().ok())
        .collect();
    assert_eq!(
        told.len(),
        written.len(),
        "tex answered about {} of {} dimensions",
        told.len(),
        written.len()
    );

    for (dimension, want) in written.iter().zip(told.iter()) {
        assert_eq!(
            texrs::special::dimension(dimension),
            Some(*want),
            "{dimension}: tex says {want}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
