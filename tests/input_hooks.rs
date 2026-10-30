//! The engine's side of LuaTeX's input callbacks (`texrs::input_hooks`):
//! each is consulted where luatex consults it, and a run with none installed
//! reads as before. No TeX installation is needed.

mod common;

use texrs::input_hooks;

#[test]
fn process_input_buffer_sees_each_line_of_the_document_without_its_end() {
    if !common::tex_tree() {
        return;
    }
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = seen.clone();
    input_hooks::set_process_input_buffer(Some(Box::new(move |line: &str| {
        log.borrow_mut().push(line.to_string());
        // The callback's answer replaces the line.
        Some(line.replace("left", "right"))
    })));
    let got = texrs::run_messages_tex(
        "\\catcode`\\{=1 \\catcode`\\}=2\n\\message{left}\n\\message{x}   \n\\end\n",
    );
    input_hooks::set_process_input_buffer(None);
    assert_eq!(got.unwrap(), "right x");
    // Every line, as it is in the file and without its line end; the
    // trailing spaces are still there, since tex drops them after.
    assert_eq!(
        seen.borrow()[..3],
        [
            "\\catcode`\\{=1 \\catcode`\\}=2".to_string(),
            "\\message{left}".to_string(),
            "\\message{x}   ".to_string()
        ]
    );
}

#[test]
fn a_line_processed_into_a_catcode_change_changes_the_next_line() {
    if !common::tex_tree() {
        return;
    }
    // The callback runs as each line is reached, so what an earlier line did
    // is in force when a later one is read -- not one pass over the file.
    input_hooks::set_process_input_buffer(Some(Box::new(|line: &str| {
        Some(line.replace("[", "{").replace("]", "}"))
    })));
    let got = texrs::run_messages_tex(
        "\\catcode`\\{=1 \\catcode`\\}=2\n\\message[a]\\catcode`\\[=1 \n\\end\n",
    );
    input_hooks::set_process_input_buffer(None);
    assert_eq!(got.unwrap(), "a");
}

#[test]
fn find_and_open_read_file_stand_in_for_the_file_system() {
    if !common::tex_tree() {
        return;
    }
    input_hooks::set_find_read_file(Some(Box::new(|id, asked: &str| {
        assert_eq!(id, 0, "\\input asks with id 0");
        Some(format!("{asked}-found.tex"))
    })));
    input_hooks::set_open_read_file(Some(Box::new(|file: &str| {
        let mut lines = vec![format!("\\message{{{file}}}")].into_iter();
        Some(Box::new(move || lines.next()) as Box<dyn FnMut() -> Option<String>>)
    })));
    let got = texrs::run_messages_tex("\\catcode`\\{=1 \\catcode`\\}=2\n\\input nowhere \\end\n");
    input_hooks::set_find_read_file(None);
    input_hooks::set_open_read_file(None);
    let got = got.unwrap();
    assert!(
        got.contains("(./nowhere-found.tex nowhere-found.tex"),
        "{got}"
    );
}
