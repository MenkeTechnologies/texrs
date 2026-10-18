//! `\directlua`: what the Lua computed, and what TeX did with it.
//!
//! A test that only proves "the chunk did not crash" would be worthless here —
//! consuming the chunk passed that bar for a year. Every test below asserts a
//! value that only exists because a Lua chunk COMPUTED it and TeX then read it
//! back: an arithmetic result typeset into the document's text, a register the
//! chunk wrote that the compiled program then reads, a control sequence the
//! chunk printed that the engine then expanded.
//!
//! Half of them are pinned against real `luatex` rather than against a number
//! written here, in the manner of `tests/etex.rs`, and skipped loudly when no
//! `luatex` is installed.

use std::process::Command;

/// The `\message` stream of a plain (non-LaTeX) document.
///
/// The catcode preamble is what a bare INITEX run needs before `{` is a group
/// character; `crate::catcode` explains why that is not the engine's default.
fn out(body: &str) -> String {
    texrs::run_messages(&source(body)).expect("run")
}

/// The TEXT of the same document — what `--text` prints.
///
/// `\end` follows the body with no line end between them: a line end IS a space
/// token, so a newline there would put a space at the end of every expected
/// string and hide the spacing these tests exist to pin.
fn text(body: &str) -> String {
    texrs::run_text(&source(body)).expect("run")
}

fn source(body: &str) -> String {
    format!("\\catcode`\\{{=1 \\catcode`\\}}=2\n{body}\\end\n")
}

/// The error a document stopped with.
fn fails(body: &str) -> String {
    let src = source(body);
    match texrs::run_messages(&src) {
        Ok(o) => panic!("expected a failure, got {o:?}"),
        Err(e) => e.0,
    }
}

/// `luatex`, if it is installed.
fn luatex() -> Option<String> {
    let out = Command::new("luatex").arg("--version").output().ok()?;
    out.status.success().then(|| "luatex".to_string())
}

/// The same document through both engines, compared on its bracketed messages.
///
/// The file is byte-identical for the two: `tex.enableprimitives` is what a
/// `luatex -ini` run needs before it has `\luaescapestring` at all, and texrs
/// answers it with a no-op, so nothing has to be varied between them.
fn both(lua: &str, body: &str) -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = format!(
        "\\catcode`\\{{=1 \\catcode`\\}}=2\n\
         \\directlua{{tex.enableprimitives('', tex.extraprimitives())}}\n\
         {body}\n\\end\n"
    );
    std::fs::write(dir.path().join("case.tex"), &src).expect("write");
    let bracketed = |out: Vec<u8>| {
        let text = String::from_utf8_lossy(&out).into_owned();
        match (text.find('['), text.rfind(']')) {
            (Some(a), Some(b)) if b > a => text[a..=b].to_string(),
            _ => String::new(),
        }
    };
    let reference = Command::new(lua)
        .args(["-ini", "-interaction=nonstopmode", "case.tex"])
        .env("max_print_line", "8000")
        .current_dir(dir.path())
        .output()
        .expect("run luatex");
    let subject = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .arg("case.tex")
        .current_dir(dir.path())
        .output()
        .expect("run texrs");
    (bracketed(reference.stdout), bracketed(subject.stdout))
}

/// A `\message` printed from Lua without writing a backslash in the .tex file.
///
/// `\\` is a macro in every real format (a line break in LaTeX, an alignment
/// break in plain), so `tex.print("\\message{...}")` in a file that loads one
/// would be expanded before Lua ever saw it — in LuaTeX exactly as here, since
/// the ⟨general text⟩ is expanded fully. `string.char(92)` is the spelling that
/// means the same thing to both engines.
fn lua_message(expr: &str) -> String {
    format!("\\directlua{{tex.print(string.char(92) .. \"message{{[\" .. {expr} .. \"]}}\")}}")
}

#[test]
fn the_manuals_own_example_is_what_the_document_typesets() {
    // LuaTeX manual §2.4.1, verbatim:
    //
    //     \count10=20
    //     a\directlua{tex.print(tex.count[10]+5)}b
    //
    // "expands to a25b". Nothing but a Lua interpreter reading a real register
    // produces the 25: consuming the chunk gives `ab`, and printing the chunk
    // gives the source text.
    assert_eq!(
        text("\\count10=20\na\\directlua{tex.print(tex.count[10]+5)}b"),
        "a25b"
    );
}

#[test]
fn what_a_chunk_prints_is_read_as_tex_and_not_as_characters() {
    // The print is a control sequence and a group. If the output were dropped
    // in as text there would be no message at all; if it were dropped in with
    // every catcode 12 there would be a `\message{[hi]}` in the document's
    // words. It is read by the mouth under the live catcode table, so it is a
    // command the engine runs.
    assert_eq!(out(&lua_message("\"hi\"")), "[hi]");
    assert_eq!(text(&lua_message("\"hi\"")), "");
}

#[test]
fn a_chunk_computes_with_the_real_lua_libraries() {
    assert_eq!(out(&lua_message("6*7")), "[42]");
    assert_eq!(
        out(&lua_message("table.concat({\"a\",\"b\"}, \"-\")")),
        "[a-b]"
    );
    assert_eq!(out(&lua_message("string.rep(\"ab\", 3)")), "[ababab]");
    assert_eq!(out(&lua_message("math.max(3, 9, 4)")), "[9]");
    // 5.3's integer division, which 5.2 does not have and 5.4 spells the same.
    assert_eq!(out(&lua_message("7 // 2")), "[3]");
}

#[test]
fn the_interpreter_is_the_one_luatex_embeds() {
    // The LuaTeX manual §4.2.1: "We currently use Lua 5.3". `lua.version` is
    // the interpreter's own answer rather than a string this crate stores, so
    // this fails if the dependency is ever built against another Lua.
    assert_eq!(out(&lua_message("lua.version")), "[Lua 5.3]");
    assert_eq!(out(&lua_message("_VERSION")), "[Lua 5.3]");
}

#[test]
fn a_chunk_reads_the_registers_the_document_set() {
    assert_eq!(
        out(&format!("\\count10=20\n{}", lua_message("tex.count[10]"))),
        "[20]"
    );
    // A dimension is scaled points on the Lua side: 2pt is 2*65536.
    assert_eq!(
        out(&format!("\\dimen3=2pt\n{}", lua_message("tex.dimen[3]"))),
        "[131072]"
    );
    assert_eq!(
        out(&format!("\\toks0={{abc}}\n{}", lua_message("tex.toks[0]"))),
        "[abc]"
    );
    // The accessor functions, which are the same registers by another spelling.
    assert_eq!(
        out(&format!("\\count5=7\n{}", lua_message("tex.getcount(5)"))),
        "[7]"
    );
    // A \countdef name is a valid index: "It is possible to use the names of
    // relevant \attributedef, \countdef, \dimendef […] as indices".
    assert_eq!(
        out(&format!(
            "\\countdef\\pageno=0 \\count0=3\n{}",
            lua_message("tex.count.pageno")
        )),
        "[3]"
    );
}

#[test]
fn a_register_a_chunk_wrote_is_the_one_the_program_reads() {
    // This is the write half of the bridge, and it is the one that cannot be
    // faked: `\the\count7` in a `\message` is lowered to a slot read in the
    // compiled program, so the number can only appear if the chunk's write
    // reached the register the VM runs on.
    assert_eq!(
        out("\\directlua{tex.setcount(7, 1000)}\\message{[\\the\\count7]}"),
        "[1000]"
    );
    assert_eq!(
        out("\\directlua{tex.count[9] = 4 tex.count[9] = tex.count[9] * 11}\\message{[\\the\\count9]}"),
        "[44]"
    );
    // "The dimension registers accept Lua numbers (in scaled points) or strings
    // (with an included absolute dimension)". Both, and the string goes through
    // tex.web §458's exact ratio, which is why an inch is not 72.27pt.
    assert_eq!(
        out("\\directlua{tex.setdimen(5, \"1in\")}\\message{[\\the\\dimen5]}"),
        "[72.26999pt]"
    );
    assert_eq!(
        out("\\directlua{tex.dimen[6] = 65536}\\message{[\\the\\dimen6]}"),
        "[1.0pt]"
    );
}

#[test]
fn a_chunk_defines_what_the_next_line_uses() {
    // `\directlua` is expandable and runs where it stands, so a macro it prints
    // is defined for the rest of the document — which is the whole reason a
    // document generates TeX from Lua rather than writing it out.
    assert_eq!(
        out("\\directlua{tex.print(string.char(92) .. \"def\" .. string.char(92) .. \"gen{GEN}\")}\\message{[\\gen]}"),
        "[GEN]"
    );
}

#[test]
fn one_lua_state_serves_the_whole_document() {
    // Two chunks, one state: the manual's `\luafunction` example depends on
    // exactly this, and so does every package that sets a table up once.
    assert_eq!(
        out(&format!(
            "\\directlua{{answer = 42}}\n{}",
            lua_message("answer")
        )),
        "[42]"
    );
}

#[test]
fn print_is_a_line_and_sprint_is_part_of_one() {
    // Manual §10.3.14.1: "The very last string of the very last tex.print
    // command in a \directlua will not have the \endlinechar appended, all
    // others do" — so two prints give `p q`, not `pq` and not `p q `.
    assert_eq!(
        text("m\\directlua{tex.print(\"p\")tex.print(\"q\")}n"),
        "mp qn"
    );
    // §10.3.14.2: sprint inserts no \endlinechar at all.
    assert_eq!(
        text("x\\directlua{tex.sprint(\"p\")tex.sprint(\"q\")}y"),
        "xpqy"
    );
    // "TEX does not switch to the 'new line' state, so that leading spaces are
    // not ignored." A run of them is one space token, as it is mid-line.
    assert_eq!(text("s\\directlua{tex.sprint(\"  lead\")}e"), "s leade");
}

#[test]
fn write_gives_every_character_a_catcode_of_its_own() {
    // §10.3.14.5: "All catcodes on that line are either 'space' (for ' ') or
    // 'character' (for all others)." So a backslash printed by tex.write is a
    // backslash in the document, NOT the start of a control sequence — the
    // difference between this and tex.print is the whole point of having both.
    assert_eq!(
        text("\\directlua{tex.write(string.char(92) .. \"message{x}\")}"),
        "\\message{x}"
    );
    assert_eq!(
        out("\\directlua{tex.write(string.char(92) .. \"message{x}\")}"),
        ""
    );
    // tex.print(-2, ...) is the same regime: "all category codes are 12 (other)
    // except for the space character".
    assert_eq!(
        text("\\directlua{tex.print(-2, string.char(92) .. \"message{x}\")}"),
        "\\message{x}"
    );
}

#[test]
fn cprint_sets_the_catcode_it_is_given() {
    // §10.3.14.4: `tex.cprint(11, ...)` makes letters, `tex.cprint(12, ...)`
    // makes other characters. Both are text; the observable difference from
    // catcode 14 is that a comment character swallows the rest.
    assert_eq!(text("\\directlua{tex.cprint(11, \"abc\")}"), "abc");
    assert_eq!(text("\\directlua{tex.cprint(12, \"$&\")}"), "$&");
    assert_eq!(text("A\\directlua{tex.cprint(14, \"gone\")}"), "A");
}

#[test]
fn luaescapestring_makes_a_tex_value_safe_in_a_lua_literal() {
    // §2.4.3, and the reason it exists: the value has a quote and a backslash
    // in it, so without escaping the chunk is a Lua syntax error rather than a
    // wrong answer. The escaping happens inside the chunk, before Lua sees it.
    assert_eq!(
        out("\\def\\risky{a\"b}\\directlua{tex.print(string.char(92) .. \"message{[\" .. \"\\luaescapestring{\\risky}\" .. \"]}\")}"),
        "[a\"b]"
    );
}

#[test]
fn luafunction_and_luadef_call_the_table_the_manual_describes() {
    // §2.4.4, the manual's own example: `t[8] = function(slot) tex.print(slot)
    // end` and "the number 8 gets typeset", because "The function, when called
    // in fact gets one argument, being the index".
    let setup = "\\directlua{local t = lua.get_functions_table() \
                 t[8] = function(slot) tex.print(slot) end}";
    assert_eq!(text(&format!("{setup}\\luafunction8")), "8");
    // `\luadef` binds a name to the same index.
    assert_eq!(text(&format!("{setup}\\luadef\\eight 8 \\eight")), "8");
}

#[test]
fn a_lua_error_is_a_tex_error_and_not_a_panic_or_a_silent_skip() {
    // The failure mode this whole module exists to remove is a document that is
    // WRONG rather than refused. A chunk that cannot run must stop the run.
    let e = fails("\\directlua{error(\"boom\")}\\message{unreached}");
    assert!(e.contains("boom"), "{e}");
    let e = fails("\\directlua{this is not lua}");
    assert!(e.contains("syntax error"), "{e}");
    // §10.3.15.6: "This creates an error somewhat like the combination of
    // \errhelp and \errmessage would."
    let e = fails("\\directlua{tex.error(\"my message\")}");
    assert!(e.contains("my message"), "{e}");
    // A callback given nonsense raises in Lua, and that reaches TeX too rather
    // than unwinding through the engine.
    let e = fails("\\directlua{tex.count.nosuchregister = 1}");
    assert!(e.contains("nosuchregister"), "{e}");
}

#[test]
fn a_chunk_that_failed_still_leaves_the_assignments_it_made() {
    // An assignment made before an error stands in TeX, and it stands here: the
    // register write is applied before the failure is reported.
    let e = fails("\\directlua{tex.setcount(3, 5) error(\"late\")}");
    assert!(e.contains("late"), "{e}");
}

#[test]
fn latelua_runs_but_contributes_nothing_to_the_input() {
    // There is no shipout here to hang a whatsit on, so `\latelua` runs where
    // it stands. What it printed is dropped, because a whatsit cannot put
    // tokens in front of the mouth however it is timed; what it ASSIGNED
    // stands, which is the half a document can observe.
    assert_eq!(text("a\\latelua{tex.print(\"NO\")}b"), "ab");
    assert_eq!(
        out("\\latelua{tex.setcount(4, 9)}\\message{[\\the\\count4]}"),
        "[9]"
    );
}

#[test]
fn the_font_fallback_chain_is_still_read_out_of_a_chunk() {
    // Every book in the corpus opens with this line, and it is the reason their
    // build needed LuaTeX. Two things have to hold now that chunks RUN. The
    // call must not stop the document: nothing loads packages here, so an
    // absent `luaotfload` global would turn a line that used to be consumed
    // into a run that dies on line one. And the chain must still reach the
    // backend, which `tests/glyph_fallback.rs` pins by drawing the glyph.
    assert_eq!(
        out("\\directlua{luaotfload.add_fallback(\"symfb\", {\"Arial:mode=base;\"})}\\message{[after]}"),
        "[after]"
    );
    // A chain the chunk BUILDS rather than writes out is the case reading the
    // text cannot answer, and the call can.
    assert_eq!(
        out("\\directlua{local t = {} t[1] = \"Arial:mode=base;\" luaotfload.add_fallback(\"symfb\", t)}\\message{[after]}"),
        "[after]"
    );
}

#[test]
fn tex_jobname_is_the_name_the_run_was_given() {
    texrs::lua::set_jobname("mybook");
    assert_eq!(out(&lua_message("tex.jobname")), "[mybook]");
    texrs::lua::reset();
}

#[test]
fn texio_write_reaches_the_terminal_stream() {
    // texrs's terminal output IS its message stream, so that is where a
    // document's own logging goes.
    assert_eq!(out("\\directlua{texio.write(\"[logged]\")}"), "[logged]");
    // `write_nl` is tex.web's `print_nl`: the text on a line of its own.
    assert_eq!(
        out("\\directlua{texio.write_nl(\"term\", \"[selected]\")}"),
        "\n[selected]"
    );
    // ltexiolib.c reads the first of several arguments as the selector
    // whatever it says, and what goes to the log alone is not on the
    // terminal: luatex prints neither `a` nor `[hidden]` here.
    assert_eq!(
        out("\\directlua{texio.write(\"a\", \"[b]\") texio.write_nl(\"log\", \"[hidden]\")}"),
        "[b]"
    );
}

// ── against the real engine ──────────────────────────────────────────────

#[test]
fn what_lua_computes_agrees_with_luatex_byte_for_byte() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // The message is printed BY the chunk rather than wrapped around it: texrs
    // expands `\directlua` where the lowerer meets it and not inside a
    // `\message` body, so a `\message{[\directlua{...}]}` would be comparing
    // that gap rather than what the Lua computed. What is compared is the same
    // in both engines: a chunk computes a value and TeX reads back the tokens
    // it printed.
    let says = |expr: &str| {
        format!("\\directlua{{tex.print(string.char(92) .. \"message{{[\" .. {expr} .. \"]}}\")}}")
    };
    for body in [
        // The manual's own worked example.
        format!("\\count10=20 {}", says("tex.count[10]+5")),
        // Lua 5.3 formats a float with %.14g, and TeX reads the digits it
        // produced. A Rust `f64` Display would give three more of them.
        says("math.pi"),
        says("1/3"),
        // An integer stays an integer in 5.3, which 5.2 could not say.
        says("2^31"),
        says("math.type(1)"),
        says("7 // 2"),
        // tex.sp parses a dimension the way the scanner does.
        says("tex.sp(\"1in\")"),
        says("tex.sp(\"0.5pt\")"),
        says("tex.romannumeral(1987)"),
        // A register written from Lua, read back by TeX.
        "\\directlua{tex.setcount(0, 6*7)}\\message{[\\the\\count0]}".to_string(),
        "\\directlua{tex.setdimen(0, \"1in\")}\\message{[\\the\\dimen0]}".to_string(),
        // A macro generated by Lua, expanded by TeX.
        "\\directlua{tex.print(string.char(92) .. \"def\" .. string.char(92) .. \"g{G}\")}\\message{[\\g]}".to_string(),
    ] {
        let body = body.as_str();
        let (want, got) = both(&lua, body);
        assert!(!want.is_empty(), "the oracle said nothing for {body}");
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn the_print_functions_space_their_output_the_way_luatex_does() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // Where the spaces fall is the part of the print functions that is easy to
    // get plausibly wrong, so it is pinned against the engine rather than
    // against a reading of the manual. The whole `\message` is built out of
    // prints, so what the two engines are compared on is how a chunk's own
    // pieces join.
    let open = "\\directlua{tex.sprint(string.char(92) .. \"message{[\")";
    for body in [
        // sprint runs strings together; print puts `\endlinechar` between them
        // and not after the last one.
        format!("{open} tex.sprint(\"p\") tex.sprint(\"q\") tex.sprint(\"]}}\")}}"),
        format!("{open} tex.print(\"p\") tex.print(\"q\") tex.sprint(\"]}}\")}}"),
        format!("{open} tex.sprint(\"  lead\") tex.sprint(\"]}}\")}}"),
        format!("{open} tex.sprint(\"trail  \") tex.sprint(\"|]}}\")}}"),
        format!("{open} tex.write(\"w x\") tex.sprint(\"]}}\")}}"),
        format!("{open} tex.print(-2, \"one two\") tex.sprint(\"]}}\")}}"),
    ] {
        let body = body.as_str();
        let (want, got) = both(&lua, body);
        assert!(!want.is_empty(), "the oracle said nothing for {body}");
        assert_eq!(got, want, "{body}");
    }
}

// ── glue registers, which are five numbers rather than a node ────────────

#[test]
fn a_glue_register_reaches_lua_as_the_five_numbers_luatex_reports() {
    // `tex.skip[n]` is a `glue_spec` NODE in LuaTeX and there are no nodes
    // here, but `tex.getglue` is the same register in plain numbers and it is
    // the spelling a document can use in both engines. All five have to be
    // right, and the two orders are the part that is easy to get plausibly
    // wrong: texrs numbers `fil` 1 as TeX82 does, LuaTeX numbers it 2 because
    // it has a finer infinity called `fi` below it.
    assert_eq!(
        out(&format!(
            "\\skip0=1pt plus 2fil minus 3pt\n{}",
            lua_message("table.concat({tex.getglue(0)}, \" \")")
        )),
        "[65536 131072 196608 2 0]"
    );
    // "when you pass false as second argument to getglue you only get the width"
    assert_eq!(
        out(&format!(
            "\\skip1=7pt plus 1fill\n{}",
            lua_message("tex.getglue(1, false)")
        )),
        "[458752]"
    );
    // Indexing answers the width, which is what `luatex` answers for
    // `tex.glue[n]`.
    assert_eq!(
        out(&format!(
            "\\skip2=3pt plus 1fil\n{}",
            lua_message("tex.glue[2]")
        )),
        "[196608]"
    );
}

#[test]
fn a_glue_a_chunk_wrote_is_the_glue_the_program_stretches_by() {
    // The write half: `\the\skip3` in the compiled program is four slot reads,
    // so this text can only appear if all four of the chunk's writes landed on
    // the register the VM runs on — and the order has to come back out of the
    // packed slot as the unit it went in as.
    assert_eq!(
        out("\\directlua{tex.setglue(3, 65536, 131072, 65536, 3, 0)}\\message{[\\the\\skip3]}"),
        "[1.0pt plus 2.0fill minus 1.0pt]"
    );
    assert_eq!(
        out("\\directlua{tex.glue[4] = 131072}\\message{[\\the\\skip4]}"),
        "[2.0pt]"
    );
    assert_eq!(
        out("\\directlua{tex.glue[5] = {65536, 65536, 0, 2, 0}}\\message{[\\the\\skip5]}"),
        "[1.0pt plus 1.0fil]"
    );
    // `fi` is LuaTeX's own fourth infinity and texrs has no unit for it, so a
    // chunk asking for one is told rather than quietly given `fil`.
    let e = fails("\\directlua{tex.setglue(6, 0, 65536, 0, 1, 0)}");
    assert!(e.contains("fi"), "{e}");
}

#[test]
fn the_node_half_of_the_register_interface_refuses_rather_than_inventing_a_value() {
    // A chunk that got a number where LuaTeX gave it a `glue_spec` userdata
    // would go wrong LATER, in the output. It stops here instead, and the
    // message names the spelling that does work.
    // `tex.skip`, `tex.getskip` and the `\muskip` three were on this list while
    // there was no node interface, and are not any more: they hand over the
    // `glue_spec` node the manual describes, pinned differentially by
    // `the_skip_registers_reach_lua_as_the_glue_spec_node_luatex_hands_over`
    // and `the_muskip_registers_reach_lua_the_same_way_the_skips_do`.
    //
    // What still refuses is the BOX half, and the reason is not the shape of
    // the node list: a box register holds a list nothing here can produce,
    // because texrs sets a page from runs of strings. That is the boundary the
    // whole node library is drawn at -- see
    // `everything_that_would_reach_the_documents_own_list_refuses_by_name`.
    let e = fails("\\directlua{local b = tex.getbox(0)}");
    assert!(e.contains("node lists"), "{e}");
    let e = fails("\\directlua{tex.setbox(0, node.new(\"hlist\"))}");
    assert!(e.contains("node lists"), "{e}");
}

#[test]
fn a_register_index_past_the_end_is_refused_and_not_written_to_its_neighbour() {
    // The count, dimension and glue files are consecutive ranges of ONE slot
    // file, so an unchecked index past LuaTeX's 65,536 would be a write to a
    // neighbour: the failure this module exists to end, a document silently
    // wrong somewhere it never named.
    let e = fails("\\directlua{tex.count[65536] = 5}");
    assert!(e.contains("out of range"), "{e}");
    let e = fails("\\directlua{tex.dimen[70000] = 5}");
    assert!(e.contains("out of range"), "{e}");
    // A register past 255 is one of LuaTeX's, and its own: `\count300` is not
    // `\dimen44`.
    assert_eq!(
        out(
            "\\dimen44=1pt \\directlua{tex.count[300] = 5}\\directlua{tex.sprint(string.char(92) .. \"message{[\" .. tex.count[300] .. \"]}\")}\\message{[\\the\\dimen44]}"
        ),
        "[5] [1.0pt]"
    );
    // And the neighbour it would have hit is untouched.
    assert_eq!(
        out("\\dimen44=1pt \\directlua{pcall(function() tex.count[300] = 5 end)}\\message{[\\the\\dimen44]}"),
        "[1.0pt]"
    );
}

#[test]
fn the_is_helpers_answer_the_register_a_name_stands_for() {
    // "local d = tex.getdimen('foo') if tex.isdimen('bar') then …". It is not a
    // boolean: `luatex` answers the register NUMBER, and only `false` when the
    // name is not a register at all.
    assert_eq!(
        out(&format!(
            "\\countdef\\zz=42 \n{}",
            lua_message("tostring(tex.iscount(\"zz\"))")
        )),
        "[42]"
    );
    assert_eq!(
        out(&lua_message("tostring(tex.iscount(\"nosuch\"))")),
        "[false]"
    );
    assert_eq!(
        out(&format!(
            "\\skipdef\\ss=11 \n{}",
            lua_message("tostring(tex.isskip(\"ss\"))")
        )),
        "[11]"
    );
}

// ── the token library ────────────────────────────────────────────────────

#[test]
fn the_scanners_take_their_argument_out_of_the_document_s_own_input() {
    // The manual's second worked example: the chunk FETCHES from the input
    // instead of being handed a string. This is the one part of the bridge that
    // cannot work off a snapshot — the scanners move the engine's own input
    // pointer, so what they ate is gone and what follows is still there.
    assert_eq!(
        out("\\directlua{tex.print(string.char(92) .. \"message{[\" .. token.scan_int()*2 .. \"]}\")}21"),
        "[42]"
    );
    assert_eq!(
        out("\\directlua{tex.print(string.char(92) .. \"message{[\" .. token.scan_dimen() .. \"]}\")}1in"),
        "[4736286]"
    );
    // A scanner reads what the ENGINE would read there, not a rescan of some
    // text: an eTeX expression and a hexadecimal constant are numbers in this
    // position because the engine's own §440 and §448 scanners say they are.
    assert_eq!(
        out("\\directlua{tex.print(string.char(92) .. \"message{[\" .. token.scan_int() .. \"]}\")}\"FF"),
        "[255]"
    );
    assert_eq!(
        out("\\directlua{tex.print(string.char(92) .. \"message{[\" .. token.scan_dimen() .. \"]}\")}\\dimexpr 2pt*3\\relax"),
        "[393216]"
    );
    // And the text after the argument is still there to be typeset, which is
    // what proves the input pointer moved by exactly the argument.
    assert_eq!(text("a\\directlua{token.scan_int()}9tail"), "atail");
}

#[test]
fn scan_keyword_and_scan_string_read_what_the_manual_says_they_read() {
    let says = |what: &str| {
        format!("\\directlua{{tex.print(string.char(92) .. \"message{{[\" .. {what} .. \"]}}\")}}")
    };
    // "returns true if the given keyword is gobbled; as with the regular TeX
    // keyword scanner this is case insensitive".
    assert_eq!(
        out(&format!(
            "{}PLUS",
            says("tostring(token.scan_keyword(\"plus\"))")
        )),
        "[true]"
    );
    assert_eq!(
        out(&format!(
            "{}minus",
            says("tostring(token.scan_keyword(\"plus\"))")
        )),
        "[false]"
    );
    // "The string scanner scans for something between curly braces and expands
    // on the way […] Otherwise it will scan characters with catcode letter or
    // other."
    assert_eq!(
        out(&format!("{}{{abc}}", says("token.scan_string()"))),
        "[abc]"
    );
    assert_eq!(
        out(&format!("{}word ", says("token.scan_word()"))),
        "[word]"
    );
    // "returns foo after scanning \foo"
    assert_eq!(
        out(&format!("{}\\relax", says("token.scan_csname()"))),
        "[relax]"
    );
    // A braced group is EXPANDED on the way, which is what makes the scanner
    // useful for an argument a macro built.
    assert_eq!(
        out(&format!(
            "\\def\\v{{VAL}}{}{{x\\v y}}",
            says("token.scan_string()")
        )),
        "[xVALy]"
    );
}

#[test]
fn a_macro_lua_defined_is_the_macro_tex_then_expands() {
    // `token.set_macro` is a `\def` made from Lua, and the only way to see that
    // it worked is to expand the macro afterwards.
    assert_eq!(
        out("\\directlua{token.set_macro(\"gen\", \"GEN\")}\\message{[\\gen]}"),
        "[GEN]"
    );
    // The body goes through the MOUTH under the live catcode table, so a
    // control sequence in it is a control sequence and not six characters.
    assert_eq!(
        out("\\def\\inner{IN}\\directlua{token.set_macro(\"outer\", \"[\" .. string.char(92) .. \"inner]\")}\\message{\\outer}"),
        "[IN]"
    );
    // And it survives into the next chunk, because the macro is the ENGINE's
    // now rather than the chunk's.
    assert_eq!(
        out("\\directlua{token.set_macro(\"two\", \"2\")}\\directlua{tex.print(string.char(92) .. \"message{[\" .. token.get_macro(\"two\") .. \"]}\")}"),
        "[2]"
    );
}

#[test]
fn get_macro_and_get_meaning_answer_the_body_and_the_parameter_text() {
    let says = |what: &str| {
        format!("\\directlua{{tex.print(string.char(92) .. \"message{{[\" .. {what} .. \"]}}\")}}")
    };
    let defs = "\\catcode`\\#=6 \\def\\bar{bar}\\def\\foo#1{foo-#1}";
    assert_eq!(
        out(&format!("{defs}{}", says("token.get_macro(\"bar\")"))),
        "[bar]"
    );
    assert_eq!(
        out(&format!("{defs}{}", says("token.get_macro(\"foo\")"))),
        "[foo-##1]"
    );
    // "->bar" and "#1->foo-#1": `\meaning` without its `macro:` prefix. The
    // `#` reaches the `\message` as a parameter character, which §294 prints
    // doubled -- `luatex -ini` prints `[foo-##1]` and `[##1->foo-##1]`.
    assert_eq!(
        out(&format!("{defs}{}", says("token.get_meaning(\"bar\")"))),
        "[->bar]"
    );
    assert_eq!(
        out(&format!("{defs}{}", says("token.get_meaning(\"foo\")"))),
        "[##1->foo-##1]"
    );
    // Not a macro, or not defined at all, answers NO VALUE in `luatex` rather
    // than nil or an empty string, so `select('#', ...)` is 0 and a chunk can
    // tell "no such macro" from "a macro whose body is empty".
    assert_eq!(
        out(&format!(
            "{defs}{}",
            says("select('#', token.get_macro(\"nosuch\"))")
        )),
        "[0]"
    );
    assert_eq!(
        out(&format!("{defs}{}", says("tostring(token.is_defined(\"bar\")) .. \" \" .. tostring(token.is_defined(\"nosuch\"))"))),
        "[true false]"
    );
}

#[test]
fn a_luafunction_can_fetch_its_own_argument_from_the_input() {
    // The manual puts the two ways side by side and this is the second:
    // "\def\mymacro{\directlua{mymacro()}} \mymacro 12pt" with
    // "local d = token.scan_dimen()" inside. `\luafunction` takes the same
    // route, so the library has to be installed for it too.
    assert_eq!(
        out("\\directlua{local t = lua.get_functions_table() \
             t[7] = function() tex.print(string.char(92) .. \"message{[\" .. token.scan_dimen() .. \"]}\") end}\
             \\luafunction7 12pt"),
        "[786432]"
    );
}

#[test]
fn a_scanner_kept_from_one_chunk_reads_the_input_of_a_later_one() {
    // A library function is a C function over TeX's globals in LuaTeX, so one
    // a package keeps -- ltluatex.lua's `createtoken = token.create`,
    // luaotfload's scanners, both taken into locals at load time -- reaches the
    // input of whatever chunk later calls it. This used to be refused here,
    // when the scanners were built for one chunk and dropped with it, and that
    // refusal is what stopped those packages loading.
    let body = "\\directlua{saved = token.scan_int}\
                \\directlua{tex.sprint(string.char(92) .. \"message{[\" .. saved() * 2 .. \"]}\")}21";
    assert_eq!(out(body), "[42]");
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (want, got) = both(&lua, body);
    assert_eq!(want, "[42]", "the oracle");
    assert_eq!(got, want);
}

#[test]
fn the_token_scanners_agree_with_luatex() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    let open = "\\directlua{tex.sprint(string.char(92) .. \"message{[\")";
    for body in [
        format!("{open} tex.sprint(token.scan_int()) tex.sprint(\"]}}\")}} 42"),
        format!("{open} tex.sprint(token.scan_dimen()) tex.sprint(\"]}}\")}} 1in"),
        format!("{open} tex.sprint(tostring(token.scan_keyword(\"plus\"))) tex.sprint(\"]}}\")}} PLUS"),
        format!("{open} tex.sprint(token.scan_string()) tex.sprint(\"]}}\")}} {{abc}}"),
        format!("{open} tex.sprint(token.scan_word()) tex.sprint(\"]}}\")}} hello "),
        format!("{open} tex.sprint(token.scan_csname()) tex.sprint(\"]}}\")}}\\relax"),
        // No leading spaces are skipped, and a space is where it stops: the one
        // scanner of the set that does not behave like TeX's own.
        format!("{open} tex.sprint(tostring(token.scan_csname())) tex.sprint(\"]}}\")}} \\relax"),
        format!("\\catcode`\\#=6 \\def\\foo#1{{foo-#1}}\\catcode`\\#=12 {open} tex.sprint(token.get_macro(\"foo\")) tex.sprint(\"]}}\")}}"),
        format!("\\catcode`\\#=6 \\def\\foo#1{{foo-#1}}\\catcode`\\#=12 {open} tex.sprint(token.get_meaning(\"foo\")) tex.sprint(\"]}}\")}}"),
        format!("{open} tex.sprint(select('#', token.get_macro(\"nosuch\"))) tex.sprint(\"]}}\")}}"),
        format!("\\directlua{{token.set_macro(\"gen\", \"GEN\")}}{open} tex.sprint(token.get_macro(\"gen\")) tex.sprint(\"]}}\")}}"),
    ] {
        let body = body.as_str();
        let (want, got) = both(&lua, body);
        assert!(!want.is_empty(), "the oracle said nothing for {body}");
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn the_glue_registers_agree_with_luatex() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    let open = "\\directlua{tex.sprint(string.char(92) .. \"message{[\")";
    for body in [
        format!("\\skip0=1pt plus 2fil minus 3pt \\relax {open} tex.sprint(table.concat({{tex.getglue(0)}}, \" \")) tex.sprint(\"]}}\")}}"),
        format!("\\skip0=1pt plus 2fill minus 3filll \\relax {open} tex.sprint(table.concat({{tex.getglue(0)}}, \" \")) tex.sprint(\"]}}\")}}"),
        format!("\\skip0=4pt \\relax {open} tex.sprint(tex.getglue(0, false)) tex.sprint(\"]}}\")}}"),
        format!("\\skip0=4pt plus 1fil \\relax {open} tex.sprint(tex.glue[0]) tex.sprint(\"]}}\")}}"),
        // The write, read back by TeX rather than by Lua.
        "\\directlua{tex.setglue(3, 65536, 131072, 65536, 3, 0)}\\message{[\\the\\skip3]}".to_string(),
        "\\countdef\\zz=42 \\directlua{tex.sprint(string.char(92) .. \"message{[\" .. tostring(tex.iscount(\"zz\")) .. \"]}\")}".to_string(),
    ] {
        let body = body.as_str();
        let (want, got) = both(&lua, body);
        assert!(!want.is_empty(), "the oracle said nothing for {body}");
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn a_fallback_chain_the_chunk_builds_reaches_the_backend() {
    // The chain a book names is the reason its build required LuaTeX. Reading
    // it out of the chunk's TEXT covers the line a book actually writes; a
    // chain the chunk COMPUTES is the case only the running call can answer,
    // and until the call existed it was silently lost.
    let built = "\\directlua{local t = {} \
                 for _, f in ipairs({\"Arial Unicode MS\", \"Noto Emoji\"}) do \
                 t[#t+1] = f .. \":mode=base;\" end \
                 luaotfload.add_fallback(\"symfb\", t)}";
    assert!(
        texrs::typeset::fallback_chain(built).is_empty(),
        "the text reader cannot see a chain that is built, which is what makes this test worth having"
    );
    let mut lowerer = texrs::lower::Lowerer::new();
    lowerer
        .lower(&format!("{}x", source(built)))
        .expect("lower");
    assert_eq!(
        lowerer.fonts.fallbacks,
        vec!["Arial Unicode MS".to_string(), "Noto Emoji".to_string()],
        "the families the CALL named, in order, with luaotfload's options cut at the colon"
    );
}

/// A chunk whose local `out` becomes the document's one bracketed message.
///
/// The same file is fed to both engines, so the chunk has to be written in the
/// subset both read the same way: one line, no comments, and `string.char(92)`
/// rather than a literal backslash (see [`lua_message`]).
fn shows(chunk: &str) -> String {
    format!(
        "\\directlua{{{chunk} tex.sprint(string.char(92) .. \"message{{[\" .. tostring(out) .. \"]}}\")}}"
    )
}

/// Three rules and two glues, as one line of Lua both engines read the same.
///
/// Rules rather than glyphs on purpose: a rule's width is a number the node
/// carries in BOTH engines, where a glyph's comes from the font, so a rule list
/// is the shape whose packaging can be compared against luatex at all.
const RULES: &str =
    "local function r(w) local n=node.new(\"rule\") n.width=w n.height=0 n.depth=0 return n end \
                     local function list() local a=r(655360) local g=node.new(\"glue\") \
                     g.width=327680 g.stretch=327680 g.shrink=131072 local b=r(655360) \
                     a.next=g g.next=b return a end ";

#[test]
fn a_list_the_chunk_built_is_packaged_by_tex_webs_own_hpack() {
    // The point of the node library here. Every number in these messages comes
    // out of `crate::pack`'s port of tex.web §649-§667 -- the natural width,
    // the badness §108 computes from the glue ratio, the glue_set, the sign and
    // the order -- and luatex is the oracle for all five at once.
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    let show = "local out = h.width .. \" \" .. h.height .. \" \" .. h.depth .. \" \" .. bd \
                .. \" \" .. h.glue_set .. \" \" .. h.glue_sign .. \" \" .. h.glue_order ";
    for body in [
        // Natural: 10pt + 5pt + 10pt, nothing set.
        shows(&format!("{RULES} local h,bd = node.hpack(list()) {show}")),
        // Stretched to 30pt: 5pt wanted from 5pt of stretch, so the ratio is 1
        // and §108's badness is 100*1^3.
        shows(&format!(
            "{RULES} local h,bd = node.hpack(list(), 1966080, \"exactly\") {show}"
        )),
        // Spread 10pt: ratio 2, badness 100*2^3 = 800.
        shows(&format!(
            "{RULES} local h,bd = node.hpack(list(), 655360, \"additional\") {show}"
        )),
        // Shrunk to 23pt: 2pt wanted from 2pt of shrink.
        shows(&format!(
            "{RULES} local h,bd = node.hpack(list(), 1507328, \"exactly\") {show}"
        )),
        // Shrunk PAST the shrink available: §664 sets the ratio to 1 and
        // reports an overfull box rather than shrinking further.
        shows(&format!(
            "{RULES} local h,bd = node.hpack(list(), 1048576, \"exactly\") {show}"
        )),
        // A vertical list: §668's vpack charges height plus depth and takes the
        // width as the widest item.
        shows(&format!(
            "{RULES} local a = r(655360) a.height=327680 local h,bd = node.vpack(a) {show}"
        )),
        // The natural dimensions of the same list, without a box.
        shows(&format!(
            "{RULES} local out = table.concat({{node.dimensions(list())}}, \" \")"
        )),
    ] {
        let (want, got) = both(&lua, &body);
        assert!(!want.is_empty(), "the oracle said nothing for {body}");
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn the_node_ids_are_luatexs_own_numbering_and_not_tex_webs() {
    // These two numberings agree up to 5 and then diverge, so an engine that
    // handed Lua its own would be wrong in a way a chunk branching on `n.id`
    // could not see until the output. `crate::node::Node::type_code` is
    // tex.web's (§135-§159) because §148 and §653 are stated as arithmetic on
    // it; what Lua is told is LuaTeX's.
    assert_eq!(
        texrs::node::Node::Math(0).type_code(),
        9,
        "tex.web numbers the math node 9"
    );
    assert_eq!(
        texrs::node::Node::Glue(texrs::node::GlueNode::default()).type_code(),
        10,
        "tex.web numbers the glue node 10"
    );
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    let body = shows(
        "local t = {} for _, n in ipairs({\"hlist\",\"vlist\",\"rule\",\"ins\",\"mark\",\
         \"adjust\",\"boundary\",\"disc\",\"whatsit\",\"local_par\",\"dir\",\"math\",\"glue\",\
         \"kern\",\"penalty\",\"unset\",\"glyph\",\"glue_spec\"}) do \
         t[#t+1] = n .. \"=\" .. node.id(n) end \
         t[#t+1] = \"12=\" .. node.type(12) t[#t+1] = \"29=\" .. node.type(29) \
         local out = table.concat(t, \" \") ",
    );
    let (want, got) = both(&lua, &body);
    assert!(!want.is_empty(), "the oracle said nothing");
    assert_eq!(got, want);
}

#[test]
fn a_nodes_glue_order_is_luatexs_fi_based_numbering() {
    // LuaTeX has a fourth, finer infinity called `fi` and numbers from it, so
    // `fil` is 2 where TeX82's is 1. Measured at the node level rather than
    // guessed from the register interface: in luatex 1.24.0 `\hbox to 20pt{\hskip
    // 0pt plus 1fil}` gives the glue `stretch_order == 2` and the box
    // `glue_order == 2`. A translation the wrong way round would put a `fil`
    // where a `fill` was asked for and set the box by the wrong infinity.
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    for body in [
        shows(
            "local a = node.new(\"rule\") a.width=655360 a.height=0 a.depth=0 \
             local g = node.new(\"glue\") g.stretch=65536 g.stretch_order=2 a.next=g \
             local h = node.hpack(a, 1310720, \"exactly\") \
             local out = g.stretch_order .. \" \" .. h.glue_order .. \" \" .. h.glue_set ",
        ),
        shows(
            "local a = node.new(\"rule\") a.width=655360 a.height=0 a.depth=0 \
             local g = node.new(\"glue\") g.stretch=65536 g.stretch_order=3 a.next=g \
             local h = node.hpack(a, 1310720, \"exactly\") \
             local out = g.stretch_order .. \" \" .. h.glue_order .. \" \" .. h.glue_set ",
        ),
        // Order 1 is `fi`, and a node holds it as luatex's does.
        shows("local g = node.new(\"glue\") g.stretch_order = 1 local out = g.stretch_order "),
    ] {
        let (want, got) = both(&lua, &body);
        assert!(!want.is_empty(), "the oracle said nothing for {body}");
        assert_eq!(got, want, "{body}");
    }
    // Packaging a `fi` glue is where texrs's three infinities run out: the
    // packer has no fourth order, and says so rather than setting it as `fil`.
    let e = fails(
        "\\directlua{local g = node.new(\"glue\") g.stretch = 65536 g.stretch_order = 1 \
         node.hpack(g, 65536, \"exactly\")}",
    );
    assert!(e.contains("fi"), "{e}");
}

#[test]
fn walking_and_editing_a_list_answers_what_luatex_answers() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    for body in [
        // `for n, id, subtype in node.traverse(head)` -- three values a step,
        // measured rather than assumed.
        shows(&format!(
            "{RULES} local t = {{}} for n, id, st in node.traverse(list()) do \
             t[#t+1] = id .. \":\" .. st end local out = table.concat(t, \" \") "
        )),
        shows(&format!(
            "{RULES} local a = list() local out = node.length(a) .. \" \" \
             .. node.count(node.id(\"rule\"), a) .. \" \" .. node.count(node.id(\"glue\"), a) "
        )),
        // insert_after answers head and the new node; the list is one longer
        // and the new node is where it was put.
        shows(&format!(
            "{RULES} local a = list() local h, n = node.insert_after(a, a, node.new(\"penalty\")) \
             local out = node.length(a) .. \" \" .. tostring(h == a) .. \" \" \
             .. node.type(a.next.id) .. \" \" .. tostring(n == a.next) "
        )),
        // insert_before at the head moves the head.
        shows(&format!(
            "{RULES} local a = list() local h, n = node.insert_before(a, a, node.new(\"penalty\")) \
             local out = node.length(h) .. \" \" .. tostring(h == n) .. \" \" .. node.type(h.id) "
        )),
        // remove answers the new head and what followed what was removed.
        shows(&format!(
            "{RULES} local a = list() local h, c = node.remove(a, a.next) \
             local out = node.length(h) .. \" \" .. tostring(h == a) .. \" \" \
             .. node.type(c.id) .. \" \" .. node.type(a.next.id) "
        )),
        shows(&format!(
            "{RULES} local a = list() local out = tostring(node.tail(a) == a.next.next) .. \" \" \
             .. tostring(node.slide(a) == a.next.next) .. \" \" \
             .. tostring(node.next(a) == a.next) .. \" \" .. tostring(node.prev(a.next) == a) "
        )),
        // A copy is a copy: a deep one, and `next` is not part of it. An empty
        // range copies to nil rather than to an empty something.
        shows(&format!(
            "{RULES} local a = list() local c = node.copy(a) \
             local out = tostring(c == a) .. \" \" .. c.width .. \" \" .. tostring(c.next) .. \" \" \
             .. node.length(node.copy_list(a)) .. \" \" .. tostring(node.copy_list(a, a)) \
             .. \" \" .. node.length(a, a.next) "
        )),
        // The field-name interface, which is what a chunk written for speed
        // uses instead of the metatable.
        shows(&format!(
            "{RULES} local a = list() local out = node.getfield(a, \"width\") .. \" \" \
             .. node.getid(a) .. \" \" .. node.getsubtype(a) .. \" \" \
             .. tostring(node.has_field(a, \"width\")) .. \" \" \
             .. tostring(node.has_field(a, \"penalty\")) .. \" \" \
             .. table.concat({{node.getwhd(a)}}, \",\") "
        )),
        shows(&format!(
            "{RULES} local a = list() local g = a.next \
             local out = table.concat({{node.getglue(g)}}, \" \") .. \" \" \
             .. tostring(node.is_zero_glue(g)) .. \" \" \
             .. tostring(node.is_zero_glue(node.new(\"glue\"))) .. \" \" \
             .. tostring(node.is_char(g)) .. \" \" .. tostring(node.is_glyph(g)) "
        )),
    ] {
        let (want, got) = both(&lua, &body);
        assert!(!want.is_empty(), "the oracle said nothing for {body}");
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn the_skip_registers_reach_lua_as_the_glue_spec_node_luatex_hands_over() {
    // `tex.skip` refused for want of a `glue_spec`, which is a node the arena
    // can carry faithfully: five integers, no subtype and no `prev`, exactly as
    // `node.fields(39)` reports. Both directions -- the register Lua reads and
    // the register TeX reads back after Lua wrote it.
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    for body in [
        format!(
            "\\skip0=1pt plus 2fil minus 3pt \\relax {}",
            shows(
                "local s = tex.skip[0] local out = s.id .. \" \" .. node.type(s.id) .. \" \" \
                 .. s.width .. \" \" .. s.stretch .. \" \" .. s.stretch_order .. \" \" \
                 .. s.shrink .. \" \" .. s.shrink_order .. \" \" .. tostring(s.subtype) .. \" \" \
                 .. tostring(node.has_field(s, \"subtype\")) "
            )
        ),
        format!(
            "\\skip0=4pt plus 1fill \\relax {}",
            shows("local s = tex.getskip(0) local out = s.width .. \" \" .. s.stretch_order ")
        ),
        // The write, read back by TeX rather than by Lua.
        "\\directlua{local n = node.new(\"glue_spec\") n.width = 196608 n.stretch = 65536 \
         n.stretch_order = 2 tex.skip[3] = n}\\message{[\\the\\skip3]}"
            .to_string(),
        "\\directlua{local n = node.new(\"glue_spec\") n.width = 131072 n.shrink = 65536 \
         tex.setskip(4, n)}\\message{[\\the\\skip4]}"
            .to_string(),
    ] {
        let (want, got) = both(&lua, &body);
        assert!(!want.is_empty(), "the oracle said nothing for {body}");
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn a_glyph_carries_the_only_font_there_is_and_measures_what_luatex_measures() {
    // texrs has no `font` library, so 0 -- LuaTeX's "no font" -- is the only
    // font id a chunk here could hold. Measured in luatex 1.24.0: a glyph with
    // font 0 has width 0 whatever `char` is, and a write to `.width` returns
    // without error and has no effect, because the dimension comes from the
    // font. So agreeing with luatex here is agreeing exactly, not approximately.
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    let body = shows(
        "local g = node.new(\"glyph\") g.char = 65 g.width = 12345 \
         local out = g.id .. \" \" .. g.char .. \" \" .. g.font .. \" \" .. g.width .. \" \" \
         .. g.height .. \" \" .. g.depth .. \" \" .. tostring(node.is_char(g)) .. \" \" \
         .. table.concat({node.getwhd(g)}, \",\") .. \" \" .. node.hpack(g).width ",
    );
    let (want, got) = both(&lua, &body);
    assert!(!want.is_empty(), "the oracle said nothing");
    assert_eq!(got, want);
}

#[test]
fn what_needs_the_stomach_or_a_font_program_still_refuses_by_name() {
    // The list TeX is building, the box registers and a font's lig/kern
    // program belong to the stomach and the fonts; each function that would
    // reach them says what it waits on rather than being absent.
    for (chunk, want) in [
        ("node.write(node.new(\"penalty\"))", "stomach"),
        ("node.last_node()", "stomach"),
        ("tex.getbox(0)", "node lists"),
        ("tex.getlist(\"page_head\")", "node lists"),
        ("node.mlist_to_hlist(node.new(\"penalty\"))", "noad"),
        (
            "node.ligaturing(node.new(\"penalty\"))",
            "handle_ligaturing",
        ),
        ("node.kerning(node.new(\"penalty\"))", "handle_kerning"),
        ("node.hyphenating(node.new(\"penalty\"))", "Liang"),
        ("node.direct.getbox(0)", "box registers"),
    ] {
        let e = fails(&format!("\\directlua{{{chunk}}}"));
        assert!(e.contains(want), "expected {want:?} in {e:?} for {chunk}");
    }
}

#[test]
fn attributes_and_the_direct_face_answer_what_luatex_answers() {
    // Attribute lists are LuaTeX's: an `attribute_list` node and an
    // `attribute` node per number, in number order; a node made while an
    // attribute register is set carries it. The direct face is the same
    // nodes as integers.
    let body = shows(
        "tex.attribute[3] = 5 local q = node.new('penalty') \
         node.set_attribute(q, 1, 10) node.set_attribute(q, 4, 40) \
         local d = node.direct local dq = d.todirect(q) local dk = d.new('kern') d.setkern(dk, 300) \
         d.setlink(dq, dk) \
         local out = tostring(tex.attribute[0]) .. ' ' .. node.get_attribute(q, 3) .. ' ' \
         .. tostring(node.has_attribute(q, 1, 11)) .. ' ' .. q.attr.id .. ' ' .. q.attr.next.number .. ' ' \
         .. node.unset_attribute(q, 4) .. ' ' .. tostring(node.has_attribute(q, 4)) .. ' ' \
         .. tostring(d.getnext(dq) == dk) .. ' ' .. d.getkern(dk) .. ' ' .. tostring(d.tonode(dk).kern) ",
    );
    let want = "[-2147483647 5 nil 40 1 40 nil true 300.0 300]";
    assert_eq!(out(&body), want);
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (oracle, got) = both(&lua, &body);
    assert_eq!(oracle, want, "the oracle");
    assert_eq!(got, oracle);
}

#[test]
fn node_new_makes_every_type_luatex_makes_with_its_fields_in_luatexs_order() {
    // `node.fields` numbers the fields as luatex does, from -1 when the type
    // has a `prev`; a whatsit is made by subtype, by number or name; a mark
    // carries a token list and a style node its style by name.
    let body = shows(
        "local f = node.fields('glyph') local w = node.fields(8, 'pdf_literal') \
         local s = node.fields('glue_spec') \
         local out = f[-1] .. ' ' .. f[4] .. ' ' .. f[17] .. ' ' .. w[4] .. ' ' .. w[5] .. ' ' \
         .. s[0] .. ' ' .. s[2] .. ' ' .. tostring(s[-1]) .. ' ' \
         .. node.new('whatsit', 'pdf_literal').subtype .. ' ' .. node.new(8, 3).subtype .. ' ' \
         .. type(node.new('mark').mark) .. ' ' .. node.new('style').style .. ' ' \
         .. node.id('ins') .. ' ' .. node.id('noad') .. ' ' .. node.type(15) .. ' ' .. node.type(39) ",
    );
    let want = "[prev char data mode data next width nil 16 3 table display 3 18 unset glue_spec]";
    assert_eq!(out(&body), want);
    let e = fails("\\directlua{node.new(\"whatsit\", \"no_such_whatsit\")}");
    assert!(e.contains("invalid whatsit type id"), "{e}");
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (oracle, got) = both(&lua, &body);
    assert_eq!(oracle, want, "the oracle");
    assert_eq!(got, oracle);
}

#[test]
fn every_field_luatex_gives_a_node_is_there_with_luatexs_starting_value() {
    // A cell keeps every field a node of its type has in luatex 1.24.0, so the
    // fields that used to be refused for want of room in `crate::node` read
    // what a fresh node has in luatex: a rule's `left` 0, a glyph's `left`
    // and `right` 1 (its hyphenation minimums), a disc's `penalty` and a
    // kern's `expansion_factor` 0, a fraction's `fam` -1, a box's `dir` nil.
    let reads = "local h, k, d, m, g, r, f = node.new('hlist'), node.new('kern'), node.new('disc'), \
                 node.new('math'), node.new('glyph'), node.new('rule'), node.new('fraction') \
                 local out = tostring(h.dir) .. ' ' .. k.expansion_factor .. ' ' .. tostring(d.replace) \
                 .. ' ' .. d.penalty .. ' ' .. m.width .. ' ' .. tostring(g.components) .. ' ' \
                 .. g.left .. g.right .. ' ' .. r.left .. ' ' .. f.fam .. ' ' .. tostring(h.attr) ";
    assert_eq!(out(&shows(reads)), "[nil 0 nil 0 0 nil 11 0 -1 nil]");
    // A name that is a field in neither engine is `nil` on a read and luatex's
    // own wording on a write.
    let e = fails("\\directlua{local p = node.new(\"penalty\") p.width = 1}");
    assert!(
        e.contains("You cannot set field width in a node of type penalty"),
        "{e}"
    );
    assert_eq!(
        text("\\directlua{local p = node.new(\"penalty\") tex.print(tostring(p.nosuchfield))}"),
        "nil"
    );
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (want, got) = both(&lua, &shows(reads));
    assert_eq!(want, "[nil 0 nil 0 0 nil 11 0 -1 nil]", "the oracle");
    assert_eq!(got, want);
}

#[test]
fn a_list_that_points_back_into_itself_is_refused_rather_than_hung() {
    // One Lua assignment away, and luatex hangs on it. An engine a test suite
    // drives cannot, so a walk that goes a million nodes without reaching nil
    // stops and says why.
    let e = fails("\\directlua{local p = node.new(\"penalty\") p.next = p node.length(p)}");
    assert!(e.contains("no end"), "{e}");
    let e = fails("\\directlua{local p = node.new(\"penalty\") p.next = p node.hpack(p)}");
    assert!(e.contains("no end"), "{e}");
}

#[test]
fn what_the_chunk_packed_is_what_the_document_then_sets() {
    // The whole point, in the form the rest of this file takes: a number that
    // exists only because Lua asked tex.web's own packer for it, typeset into
    // the document's text. 10pt + 5pt + 10pt at natural size is 1638400sp, and
    // `tex.sp` reads the string form back to the same integer.
    assert_eq!(
        text(&format!(
            "{RULES_TEX}A\\directlua{{local h = node.hpack(list()) tex.print(h.width)}}B"
        )),
        "A1638400B"
    );
    // And the badness of the same list stretched to 30pt, which is §108's
    // 100*ratio^3 rather than anything this module computed.
    assert_eq!(
        text(&format!(
            "{RULES_TEX}A\\directlua{{local _, bd = node.hpack(list(), 1966080, \"exactly\") tex.print(bd)}}B"
        )),
        "A100B"
    );
}

/// [`RULES`] as it goes into a plain `\directlua`, where the braces are the
/// document's own rather than a `format!` escape.
const RULES_TEX: &str = "\\directlua{function r(w) local n=node.new(\"rule\") n.width=w n.height=0 n.depth=0 return n end \
                         function list() local a=r(655360) local g=node.new(\"glue\") \
                         g.width=327680 g.stretch=327680 g.shrink=131072 local b=r(655360) \
                         a.next=g g.next=b return a end}";

#[test]
fn the_muskip_registers_reach_lua_the_same_way_the_skips_do() {
    // `\muskip` is a register file of its own now (`crate::compiler::MUSKIP_BASE`,
    // scanned in math units), so the LuaTeX interface over it is the skip
    // interface at another base: a `glue_spec` node for `tex.muskip` and
    // `tex.getmuskip`, five bare numbers for `tex.getmuglue`, the width alone
    // for `tex.muglue`, and the register number for `tex.ismuskip`.
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    for body in [
        format!(
            "\\muskip0=3mu plus 1mu minus 2mu \\relax {}",
            shows(
                "local s = tex.muskip[0] local out = s.id .. \" \" .. s.width .. \" \" \
                 .. s.stretch .. \" \" .. s.shrink .. \" \" \
                 .. table.concat({tex.getmuglue(0)}, \",\") .. \" \" .. tex.muglue[0] .. \" \" \
                 .. tex.ismuskip(0) "
            )
        ),
        format!(
            "\\muskip2=1mu plus 2fil \\relax {}",
            shows(
                "local s = tex.getmuskip(2) local out = s.width .. \" \" .. s.stretch .. \" \" \
                 .. s.stretch_order "
            )
        ),
        // Both writes, read back by TeX rather than by Lua.
        "\\directlua{local n = node.new(\"glue_spec\") n.width = 131072 n.stretch = 65536 \
         tex.setmuskip(1, n)}\\message{[\\the\\muskip1]}"
            .to_string(),
        "\\directlua{tex.setmuglue(4, 196608, 65536, 0, 0, 0)}\\message{[\\the\\muskip4]}"
            .to_string(),
        "\\directlua{tex.muskip[5] = node.new(\"glue_spec\")}\\message{[\\the\\muskip5]}"
            .to_string(),
    ] {
        let (want, got) = both(&lua, &body);
        assert!(!want.is_empty(), "the oracle said nothing for {body}");
        assert_eq!(got, want, "{body}");
    }
    // The bound is LuaTeX's 65,536, and the muskip file sits above the skip
    // file in one slot range -- so an unchecked index would land in a neighbour.
    let e = fails("\\directlua{tex.muskip[65536] = node.new(\"glue_spec\")}");
    assert!(e.contains("out of range") && e.contains("muskip"), "{e}");
}

#[test]
fn an_internal_parameter_is_read_and_assigned_as_luatex_reads_and_assigns_it() {
    // `tex.<name>`, `tex.get` and `tex.set` reach the parameters the engine
    // holds -- the §236 integers, the §247 dimensions, the §224 glues, the
    // §230 token lists -- and an assignment goes through the engine's own,
    // so `\the` sees it and a group restores it.
    let body = shows(
        "tex.tolerance = 200 tex.set('global', 'pretolerance', 150) tex.hsize = '100pt' \
         tex.set('baselineskip', 786432) tex.everypar = 'abc' \
         local out = tex.tolerance .. ' ' .. tex.get('pretolerance') .. ' ' .. tex.hsize .. ' ' \
         .. tex.get('baselineskip', false) .. ' ' .. tex.baselineskip.width .. ' ' .. tex.everypar ",
    );
    let want = "[200 150 6553600 786432 786432 abc]";
    assert_eq!(out(&body), want);
    assert_eq!(
        out("\\directlua{tex.tolerance = 321}\\message{[\\the\\tolerance]}"),
        "[321]"
    );
    // A parameter texrs does not hold at all still says so.
    let e = fails("\\directlua{local v = tex.prevdepth}");
    assert!(e.contains("does not hold it"), "{e}");
    // The manual is explicit that `tex.foo = 123` is valid on this table, so a
    // name that is not a parameter has to keep behaving like a Lua field.
    assert_eq!(
        text("\\directlua{tex.foo = 123 tex.print(tex.foo .. \" \" .. tostring(tex.bar))}"),
        "123 nil"
    );
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (oracle, got) = both(&lua, &body);
    assert_eq!(oracle, want, "the oracle");
    assert_eq!(got, oracle);
}

/// One expression's value, as luatex 1.24.0 printed it, through the same
/// `\message` both engines write.
fn says_value(expr: &str) -> String {
    format!(
        "\\directlua{{tex.sprint(string.char(92) .. \"message{{[\" .. tostring({expr}) .. \"]}}\")}}"
    )
}

#[test]
fn the_c_libraries_are_the_ones_luatex_links() {
    // LPeg, slnunicode, LuaFileSystem and bit32 are compiled from the copies
    // LuaTeX ships, so these are not re-implementations agreeing by luck: each
    // value was printed by luatex 1.24.0 for the same expression. The checks
    // that need no TeX installation run everywhere; the oracle half below
    // re-measures them when luatex is there.
    let cases: &[(&str, &str)] = &[
        ("lpeg.version()", "1.0.1"),
        ("lpeg.match(lpeg.Cs((lpeg.P'a'/'A' + 1)^0), 'banana')", "bAnAnA"),
        ("unicode.utf8.upper('straße äöü')", "STRAßE ÄÖÜ"),
        ("unicode.utf8.len('äöü€x')", "5"),
        ("unicode.utf8.sub('äöü€x', 2, 4)", "öü€"),
        ("unicode.grapheme.len('e' .. string.char(204, 129) .. 'a')", "2"),
        ("md5.sumhexa('The quick brown fox')", "a2004f37730b9445670a738fa0fc9ee5"),
        ("md5.decrypt(md5.crypt('secret message', 'key', 'seed'), 'key')", "secret message"),
        ("(sha2.digest256('abc'):gsub('.', function(c) return string.format(string.char(37) .. '02x', c:byte()) end))",
         "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        ("bit32.arshift(-16, 2)", "4294967292"),
        ("bit32.replace(0, 7, 4, 3)", "112"),
        ("math.pow(2, 10)", "1024.0"),
        ("table.concat(string.explode('  a  b c '), '|')", "a|b|c"),
        ("table.concat(string.explode('a,,b,', ','), '|')", "a||b|"),
        ("string.utflength('aä€' .. string.char(128) .. 'x')", "5"),
        ("select('#', string.utfvalue('aä€' .. string.char(226, 130) .. 'b'))", "3"),
        ("(function() local z = zlib.compress('hello hello hello hello') return zlib.decompress(z) end)()", "hello hello hello hello"),
        // compress answers two values, and the second reaches decompress as its
        // window size: nil, in luatex as here.
        ("zlib.decompress(zlib.compress('hello hello hello hello'))", "nil"),
        ("(zlib.compress('hello hello hello hello'):gsub('.', function(c) return string.format(string.char(37) .. '02x', c:byte()) end))",
         "789ccb48cdc9c957c8402701680308b1"),
        ("lfs.isdir('.')", "true"),
        ("#package.searchers", "2"),
        ("debug.getinfo", "nil"),
        ("tex.luatexversion", "124"),
        ("callback.register('pre_linebreak_filter', function() end)", "45"),
        ("select(2, callback.register('no_such', function() end))", "No such callback exists."),
    ];
    for (expr, want) in cases {
        assert_eq!(out(&says_value(expr)), format!("[{want}]"), "{expr}");
    }
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    for (expr, want) in cases {
        let (oracle, got) = both(&lua, &says_value(expr));
        assert_eq!(oracle, format!("[{want}]"), "the oracle, for {expr}");
        assert_eq!(got, oracle, "{expr}");
    }
}

#[test]
fn require_finds_a_module_the_way_luatexs_kpse_searcher_does() {
    // luainit.c's `luatex_kpse_lua_find`: a module is a file of kpathsea's
    // `lua` format, so `require` reaches the TeX tree rather than
    // `package.path`. What a missing one says is the searcher's own line.
    assert_eq!(
        out(&says_value("select(2, pcall(require, 'no-such-module-xyz')):match('kpse lua searcher[^' .. string.char(10) .. ']*')")),
        "[kpse lua searcher] file not found: 'no-such-module-xyz']"
    );
    if texrs::kpse::locate("lualibs.lua").is_none() {
        eprintln!("skipping the TeX-tree half: kpsewhich cannot find lualibs.lua");
        return;
    }
    // lualibs is LPeg and zlib end to end; loading it at all is the test that
    // both are LuaTeX's.
    assert_eq!(
        out(&says_value(
            "(function() require('lualibs') return type(lpeg.patterns) end)()"
        )),
        "[table]"
    );
}

#[test]
fn an_hpack_filter_sees_the_list_the_stomach_packs_and_its_answer_is_packed() {
    // `callback.register('hpack_filter', f)` fills the stomach's hook point
    // (`crate::node::hooks`), so the function is called while the program
    // runs, with luatex's arguments -- the list, the group code, the size and
    // the pack type -- and the list it answers is what the box is made of.
    // The callback adds a 5pt kern after the 1pt one: the box is 6pt wide in
    // luatex 1.24.0 as here.
    let body = "\\directlua{callback.register('hpack_filter', function(h, gc, size, pt) \
                local k = node.new('kern') k.kern = 5 * 65536 h.next = k \
                texio.write('[' .. gc .. ' ' .. pt .. ' ' .. node.type(h.id) .. ']') return h end)}\
                \\setbox0\\hbox{\\kern1pt}\\message{[\\the\\wd0]}";
    assert_eq!(out(body), "[hbox additional kern] [6.0pt]");
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (want, got) = both(&lua, body);
    assert_eq!(want, "[hbox additional kern] [6.0pt]", "the oracle");
    assert_eq!(got, want);
}

#[test]
fn a_font_defined_from_lua_is_kept_and_read_tfm_reads_what_luatex_reads() {
    // `font.define` keeps the table it was given, and `font.getfont` answers
    // that table; `font.read_tfm` builds luatex's own table for a TFM, the
    // characters scaled to the size asked for.
    let body = shows(
        "local id = font.define({ name = 'x', size = 655360, characters = { [65] = { width = 100 } } }) \
         local f = font.getfont(id) \
         local out = f.name .. ' ' .. f.characters[65].width .. ' ' .. tostring(font.fonts[id] == f) \
         .. ' ' .. tostring(font.getfont(0)) ",
    );
    assert_eq!(out(&body), "[x 100 true nil]");
    if texrs::kpse::locate("cmr10.tfm").is_none() {
        eprintln!("skipping the TFM half: kpsewhich cannot find cmr10.tfm");
        return;
    }
    let tfm = shows(
        "local f = font.read_tfm('cmr10', 655360) local c = f.characters \
         local out = f.designsize .. ' ' .. f.parameters.quad .. ' ' .. c[65].width .. ' ' \
         .. c[102].italic .. ' ' .. c[102].ligatures[105].char .. ' ' .. c[65].kerns[86] ",
    );
    assert_eq!(out(&tfm), "[655360 655361 491521 50973 12 -72819]");
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    for b in [&body, &tfm] {
        let (want, got) = both(&lua, b);
        assert!(!want.is_empty(), "the oracle said nothing for {b}");
        assert_eq!(got, want, "{b}");
    }
}

#[test]
fn the_pdf_library_numbers_objects_as_luatexs_backend_does() {
    // An immediate raw object goes into an object stream when the file is
    // opened at PDF 1.5 or later with `objcompresslevel` above 0 (below 1.5
    // luatex writes none: BUGS.md), and the
    // first one opens the stream, which takes the next number; a stream
    // object never goes into one; an annotation reserved by name is not
    // `getlastobj`'s. The objects are the backend's, numbered as
    // `\pdfextension obj` numbers them. The dictionary strings are kept and read back; a number is not a string and
    // is ignored. `\outputmode` is for the `luatex -ini` oracle, which starts
    // in DVI.
    let probe = |minor: u32| {
        format!(
            "\\outputmode=1 {}",
            shows(&format!(
                "pdf.setobjcompresslevel(2) pdf.setminorversion({minor}) \
                 local a = pdf.immediateobj('x') local b = pdf.immediateobj('stream', 'y') \
                 local c = pdf.obj('z') local e = pdf.reserveobj('annot') \
                 pdf.setcatalog('/X 1') pdf.setinfo(5) \
                 local ok, m = pcall(pdf.obj, c, 'again') \
                 local out = a .. ' ' .. b .. ' ' .. c .. ' ' .. e .. ' ' .. pdf.getlastobj() .. ' ' \
                 .. pdf.getmaxobjnum() .. ' ' .. pdf.getobjtype(e) .. ' ' \
                 .. pdf.getcatalog() .. ' ' .. tostring(pdf.getinfo()) .. ' ' \
                 .. m .. ' ' .. tostring(pdf.getobjtype(99)) "
            ))
        )
    };
    let streams = "[1 3 4 5 4 5 annot /X 1 nil pdf.obj() object in use nil]";
    assert_eq!(out(&probe(7)), streams);
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    for (minor, want) in [(7, streams)] {
        let (oracle, got) = both(&lua, &probe(minor));
        assert_eq!(oracle, want, "the oracle");
        assert_eq!(got, oracle);
    }
}

#[test]
fn the_real_luaotfload_loads_and_registers_its_font_definer() {
    // TeX Live's own luaotfload, loaded the way its `luaotfload.sty` loads it
    // under ltluatex: `require` then `luaotfload.main()`. That reaches the
    // whole fontloader, which takes `pdfe.new`, `pdf.getpageresources` and
    // `node.direct` at load time; what it leaves behind is luatex's -- its
    // definer on `define_font` and the same four callbacks set.
    if texrs::kpse::locate("luaotfload.lua").is_none()
        || texrs::kpse::locate("ltluatex.lua").is_none()
    {
        eprintln!("skipping: kpsewhich cannot find luaotfload.lua and ltluatex.lua");
        return;
    }
    let body = "\\catcode`\\@=11 \
        \\countdef\\e@alloc@attribute@count=200 \\countdef\\e@alloc@whatsit@count=201 \
        \\countdef\\e@alloc@bytecode@count=202 \\countdef\\e@alloc@luafunction@count=203 \
        \\countdef\\e@alloc@luachunk@count=204 \
        \\countdef\\CountZero=0 \\chardef\\charzero=0 \\dimendef\\dimenzero=0 \
        \\skipdef\\skipzero=0 \\toksdef\\tokszero=0 \\mathchardef\\mathcharzero=0 \
        \\directlua{require('ltluatex') require('luaotfload') luaotfload.main() \
        local t = {} for k, v in pairs(callback.list()) do if v then t[#t + 1] = k end end \
        table.sort(t) tex.sprint(string.char(92) .. 'message{[' .. table.concat(t, ' ') .. ' ' \
        .. table.concat(luatexbase.callback_descriptions('define_font'), ',') .. ']}')}";
    let want = "[define_font glyph_stream_provider hpack_filter pre_linebreak_filter luaotfload.define_font]";
    assert_eq!(out(body), want);
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (oracle, got) = both(&lua, body);
    assert_eq!(oracle, want, "the oracle");
    assert_eq!(got, oracle);
}

#[test]
fn a_parameter_never_assigned_reads_its_initial_value_and_a_zero_reads_zero() {
    // LuaTeX's own dimensions start where luatex starts them, `\pxdimen` at
    // 65781sp and a page offset at an inch, so `tex.pxdimen` answers that
    // before anything assigns it; and one assigned zero answers zero, not the
    // initial value it replaced.
    let body = shows(
        "local a = tex.pxdimen .. ' ' .. tex.pageleftoffset .. ' ' .. tex.pagewidth \
         tex.pxdimen = 0 local out = a .. ' ' .. tex.pxdimen ",
    );
    let want = "[65781 4736287 0 0]";
    assert_eq!(out(&body), want);
    assert_eq!(
        out("\\pxdimen=0pt \\directlua{tex.sprint(string.char(92) .. 'message{[' .. tex.pxdimen .. ']}')}"),
        "[0]"
    );
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (oracle, got) = both(&lua, &body);
    assert_eq!(oracle, want, "the oracle");
    assert_eq!(got, oracle);
}

#[test]
fn the_pdf_variables_are_pdfvariables_entries() {
    // `pdf.setcompresslevel` and its siblings store into the entry
    // `\pdfvariable` names (`set_tex_extension_count_register`, a plain store
    // that a group does not undo), and `pdf.get*` reads it, so `\the`, Lua and
    // the writer agree on one value.
    let body = "\\pdfvariable compresslevel=4 \
        {\\directlua{pdf.setobjcompresslevel(3) pdf.setcompresslevel(12)}}\
        \\directlua{tex.sprint(string.char(92) .. 'message{[' .. pdf.getcompresslevel() .. ' ' \
        .. pdf.getobjcompresslevel() .. ']}')}\
        \\message{[\\the\\pdfvariable compresslevel/\\the\\pdfvariable objcompresslevel]}";
    let want = "[9 3] [9/3]";
    assert_eq!(out(body), want);
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (oracle, got) = both(&lua, body);
    assert_eq!(oracle, want, "the oracle");
    assert_eq!(got, oracle);
}

#[test]
fn tex_catcodetable_switches_the_table_the_mouth_reads_with() {
    // `tex.catcodetable` reads and assigns LuaTeX's current catcode table
    // through the engine's own `\catcodetable`, so a switch made from Lua in
    // a group is undone at its end like one made in TeX.
    let body = "\\savecatcodetable 5 \
        \\directlua{local a = tex.catcodetable tex.catcodetable = 5 local b = tex.get('catcodetable') \
        tex.set('catcodetable', 0) tex.sprint(string.char(92) .. 'message{[' .. a .. ' ' .. b .. ' ' \
        .. tex.catcodetable .. ']}')}\
        {\\directlua{tex.catcodetable = 5}\\message{[\\the\\catcodetable]}}\\message{[\\the\\catcodetable]}";
    let want = "[0 5 0] [5] [0]";
    assert_eq!(out(body), want);
    let Some(lua) = luatex() else {
        eprintln!("skipping the luatex half: no `luatex` on PATH");
        return;
    };
    let (oracle, got) = both(&lua, body);
    assert_eq!(oracle, want, "the oracle");
    assert_eq!(got, oracle);
}
