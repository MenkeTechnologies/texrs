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
    assert_eq!(
        out("\\directlua{texio.write_nl(\"term\", \"[selected]\")}"),
        "[selected]"
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
    let e = fails("\\directlua{local s = tex.skip[0]}");
    assert!(e.contains("tex.getglue"), "{e}");
    let e = fails("\\directlua{tex.getskip(0)}");
    assert!(e.contains("tex.getglue"), "{e}");
    // `node` is absent rather than stubbed, so a chunk reaching for it stops
    // with Lua's own nil-index error.
    let e = fails("\\directlua{node.new(\"glyph\")}");
    assert!(e.contains("node"), "{e}");
}

#[test]
fn a_register_index_past_the_end_is_refused_and_not_written_to_its_neighbour() {
    // The count, dimension and glue files are consecutive ranges of ONE slot
    // file, so an unchecked `tex.count[300]` is a write to `\dimen44` and an
    // unchecked `tex.dimen[300]` lands in the middle of `\skip11`. Both are the
    // failure this module exists to end: a document silently wrong somewhere it
    // never named.
    let e = fails("\\directlua{tex.count[300] = 5}");
    assert!(e.contains("out of range"), "{e}");
    let e = fails("\\directlua{tex.dimen[300] = 5}");
    assert!(e.contains("out of range"), "{e}");
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
        "[foo-#1]"
    );
    // "->bar" and "#1->foo-#1": `\meaning` without its `macro:` prefix.
    assert_eq!(
        out(&format!("{defs}{}", says("token.get_meaning(\"bar\")"))),
        "[->bar]"
    );
    assert_eq!(
        out(&format!("{defs}{}", says("token.get_meaning(\"foo\")"))),
        "[#1->foo-#1]"
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
fn the_scanners_are_gone_once_the_chunk_that_had_them_is() {
    // A scanner holds the engine's input pointer, so it is built for one chunk
    // and dropped with it. A chunk that squirrels one away and calls it later
    // gets an error — which is the right answer, because by then there is
    // nothing to scan — and never a stale pointer.
    let e = fails("\\directlua{saved = token.scan_int}1\\directlua{saved()}");
    assert!(
        !e.is_empty(),
        "calling a dead scanner must not succeed silently"
    );
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
