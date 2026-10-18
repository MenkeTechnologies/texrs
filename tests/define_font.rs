//! LuaTeX's `define_font` callback, as `crate::node::hooks` carries it: a
//! font made from metrics, with no TFM file behind it, reaches both the
//! lowerer's font memory and the program's, characters above 255 included.

use texrs::node::hooks::{self, Defined};
use texrs::stomach::font::{Font, Metrics};

const PT: i64 = 65536;

fn metrics(width: i64) -> Metrics {
    Metrics {
        width,
        height: 7 * PT,
        depth: 0,
        italic: 0,
    }
}

#[test]
fn a_font_from_the_callback_sets_its_characters_wide_ones_too() {
    let mut asked = Vec::new();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = seen.clone();
    hooks::set(|h| {
        h.define_font = Some(Box::new(
            move |_eng: &mut texrs::expand::Engine,
                  _lx: &mut texrs::lexer::Lexer,
                  name: &str,
                  size: i64| {
                log.borrow_mut().push((name.to_string(), size));
                let chars = [('A' as u32, metrics(5 * PT)), (0x263A, metrics(9 * PT))];
                Some(Defined::Font(Box::new(Font::from_metrics(
                    name,
                    "",
                    12 * PT,
                    10 * PT,
                    vec![0, 3 * PT, PT, PT, 4 * PT, 12 * PT, 0],
                    chars,
                ))))
            },
        ))
    });
    let src = "\\catcode`\\{=1 \\catcode`\\}=2
               \\font\\x=madeup at 12pt \\x\\setbox0\\hbox{A\\char9786 A}\\showthe\\wd0\n\
               \\font\\y=madeup scaled 2000 \\showthe\\fontdimen6\\y\n";
    let out = texrs::run_list_path(src).map(|(m, _)| m);
    hooks::clear();
    asked.extend(seen.borrow().iter().cloned());
    let out = out.expect("runs");
    // 5 + 9 + 5: both characters, the wide one from the hash, measured.
    assert!(out.contains("> 19.0pt."), "{out}");
    // §1258's two size forms, as LuaTeX passes them.
    assert_eq!(
        asked,
        vec![
            ("madeup".to_string(), 12 * PT),
            ("madeup".to_string(), -2000)
        ]
    );
    assert!(out.contains("> 12.0pt."), "{out}");
}
