//! Running a `.bst`: the other half of BibTeX, ported from `bibtex.web`.
//!
//! [`crate::bst`] reads a style. This runs one. A `.bst` is a program in
//! Patashnik's stack language, and running it is what turns a `.bib` and a
//! `.aux` into the `.bbl` a document \input's: the style decides what a
//! reference looks like, which is why `plain` and `alpha` produce different
//! bibliographies from the same database.
//!
//! The language is a postfix stack machine with three kinds of value —
//! integers, strings and functions — and 37 builtins. Most are ordinary; four
//! carry nearly all of the format's real behaviour and are where a
//! reimplementation goes wrong:
//!
//!  * `format.name$` splits a name into First, von, Last and Jr the way
//!    `bibtex.web` §386-§399 does, and then renders it through a format string
//!    like `{vv~}{ll}{, jj}{, f.}`, including the discretionary tie that turns
//!    into `~` after something short and a space after something long.
//!  * `change.case$`, `purify$`, `text.length$` and `text.prefix$` all treat a
//!    brace group beginning with a backslash as a *special character*: one
//!    character, whatever its length, whose insides are left alone.
//!  * `width$` measures in cmr10, from the table `bibtex.web` carries.
//!  * The output is broken into lines at 79 columns with a two-space
//!    continuation indent, which is why a `.bbl` looks the way it does.
//!
//! Everything here is held against the real `bibtex` binary, whole `.bbl`
//! against whole `.bbl`, in `tests/bibtex.rs`.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::bib::{Aux, Bib, Entry};
use crate::bst::{Command, Style, Token};

/// A value on the stack.
#[derive(Debug, Clone)]
enum Val {
    Int(i64),
    Str(String),
    /// A function, pushed by name with `'name` or as a literal block.
    Fun(Rc<Vec<Token>>),
    /// A field the entry does not have. BibTeX keeps this apart from the empty
    /// string, because `missing$` and `empty$` ask different questions.
    Missing,
}

impl Val {
    fn as_int(&self) -> Option<i64> {
        match self {
            Val::Int(i) => Some(*i),
            _ => None,
        }
    }

    fn as_str(&self) -> String {
        match self {
            Val::Str(s) => s.clone(),
            Val::Int(i) => i.to_string(),
            _ => String::new(),
        }
    }
}

/// One entry's state while the style runs: its fields, and the variables the
/// style declared for it.
#[derive(Debug, Default, Clone)]
struct EntryState {
    key: String,
    kind: String,
    fields: BTreeMap<String, String>,
    ints: BTreeMap<String, i64>,
    strs: BTreeMap<String, String>,
}

/// The `.bbl` being written, broken into lines the way BibTeX breaks them.
#[derive(Default)]
struct Out {
    line: String,
    text: String,
}

/// BibTeX's `max_print_line`: a `.bbl` line is broken to fit in this.
const MAX_PRINT_LINE: usize = 79;
/// Its `min_print_line`: a break is never taken this early, so a line with no
/// space in it goes out whole rather than being cut to nothing.
const MIN_PRINT_LINE: usize = 3;

impl Out {
    fn write(&mut self, s: &str) {
        self.line.push_str(s);
    }

    /// End the line, breaking it at a space to fit, with the two-space
    /// continuation indent BibTeX uses (`bibtex.web` §325).
    fn newline(&mut self) {
        let mut rest: String = self.line.trim_end().to_string();
        self.line.clear();
        let mut indent = "";
        loop {
            let full: Vec<char> = format!("{indent}{rest}").chars().collect();
            if full.len() <= MAX_PRINT_LINE {
                self.text.extend(full);
                self.text.push('\n');
                return;
            }
            let mut at = MAX_PRINT_LINE;
            while at > MIN_PRINT_LINE && full[at] != ' ' {
                at -= 1;
            }
            if at == MIN_PRINT_LINE {
                // Nothing to break at: a long unbroken run goes out whole
                // rather than being cut in the middle of a word.
                self.text.extend(full.iter());
                self.text.push('\n');
                return;
            }
            self.text.extend(full[..at].iter());
            self.text.push('\n');
            rest = full[at + 1..].iter().collect();
            indent = "  ";
        }
    }
}

/// A style, running.
pub struct Vm<'a> {
    style: &'a Style,
    functions: BTreeMap<String, Rc<Vec<Token>>>,
    fields: BTreeSet<String>,
    entry_ints: BTreeSet<String>,
    entry_strs: BTreeSet<String>,
    global_ints: BTreeMap<String, i64>,
    global_strs: BTreeMap<String, String>,
    entries: Vec<EntryState>,
    /// The order `ITERATE` walks, which `SORT` and `REVERSE` change.
    order: Vec<usize>,
    current: Option<usize>,
    preamble: String,
    stack: Vec<Val>,
    out: Out,
    /// What BibTeX would have printed to the terminal.
    pub warnings: Vec<String>,
}

impl<'a> Vm<'a> {
    /// A machine loaded with `style`, the entries `aux` cites, and what `db`
    /// holds for them. The entries arrive in citation order, as BibTeX reads
    /// them, and stay that way until the style sorts them.
    pub fn new(style: &'a Style, db: &Bib, aux: &Aux) -> Vm<'a> {
        let mut vm = Vm {
            style,
            functions: BTreeMap::new(),
            fields: BTreeSet::new(),
            entry_ints: BTreeSet::new(),
            entry_strs: BTreeSet::new(),
            global_ints: BTreeMap::new(),
            global_strs: BTreeMap::new(),
            entries: Vec::new(),
            order: Vec::new(),
            current: None,
            preamble: db.preamble.clone(),
            stack: Vec::new(),
            out: Out::default(),
            warnings: Vec::new(),
        };

        for command in &style.commands {
            match command {
                Command::Entry {
                    fields,
                    integers,
                    strings,
                } => {
                    vm.fields
                        .extend(fields.iter().map(|f| f.to_ascii_lowercase()));
                    vm.entry_ints.extend(integers.iter().cloned());
                    vm.entry_strs.extend(strings.iter().cloned());
                }
                Command::Integers(names) => {
                    for name in names {
                        vm.global_ints.insert(name.clone(), 0);
                    }
                }
                Command::Strings(names) => {
                    for name in names {
                        vm.global_strs.insert(name.clone(), String::new());
                    }
                }
                Command::Function { name, body } => {
                    vm.functions.insert(name.clone(), Rc::new(body.clone()));
                }
                _ => {}
            }
        }
        // `sort.key$` is an entry string BibTeX declares itself, and `crossref`
        // is a field it declares itself: no style lists either in its ENTRY,
        // and every style uses both.
        vm.entry_strs.insert("sort.key$".into());
        vm.fields.insert("crossref".into());

        let selection = db.select(aux);
        for key in &aux.citations {
            let Some(entry) = selection.cited.iter().find(|e| &e.key == key) else {
                continue;
            };
            if !vm.entries.iter().any(|e| e.key == entry.key) {
                vm.entries.push(vm.state_of(entry));
            }
        }
        // `\citation{*}` asks for the whole database, in the order it was read.
        if aux.all {
            for entry in &db.entries {
                if !vm.entries.iter().any(|e| e.key == entry.key) {
                    vm.entries.push(vm.state_of(entry));
                }
            }
        }
        for key in &selection.missing {
            vm.warnings
                .push(format!("I didn't find a database entry for \"{key}\""));
        }
        vm.resolve_crossrefs(db);
        vm.order = (0..vm.entries.len()).collect();
        vm
    }

    /// What `crossref` means: an entry inherits every field it does not have
    /// from the entry it cross-references, and the entry it points at joins the
    /// bibliography only if it was cited on its own or is pointed at by at
    /// least `MIN_CROSSREFS` others. One that does not join has its `crossref`
    /// taken away from the child, so the style does not write a `\cite` to a
    /// reference nobody will find (`bibtex.web` §109).
    fn resolve_crossrefs(&mut self, db: &Bib) {
        const MIN_CROSSREFS: usize = 2;

        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &self.entries {
            if let Some(target) = entry.fields.get("crossref") {
                *counts.entry(target.trim().to_string()).or_default() += 1;
            }
        }
        // A target that is pointed at often enough joins the bibliography, at
        // the end, in the order it was first pointed at.
        for entry in self.entries.clone() {
            let Some(target) = entry.fields.get("crossref") else {
                continue;
            };
            let target = target.trim().to_string();
            if counts.get(&target).copied().unwrap_or(0) < MIN_CROSSREFS {
                continue;
            }
            if self
                .entries
                .iter()
                .any(|e| e.key.eq_ignore_ascii_case(&target))
            {
                continue;
            }
            if let Some(found) = db
                .entries
                .iter()
                .find(|e| e.key.eq_ignore_ascii_case(&target))
            {
                let state = self.state_of(found);
                self.entries.push(state);
            }
        }

        let listed: BTreeSet<String> = self.entries.iter().map(|e| e.key.clone()).collect();
        for at in 0..self.entries.len() {
            let Some(target) = self.entries[at].fields.get("crossref").cloned() else {
                continue;
            };
            let target = target.trim().to_string();
            let parent = db
                .entries
                .iter()
                .find(|e| e.key.eq_ignore_ascii_case(&target))
                .map(|e| self.state_of(e));
            match parent {
                Some(parent) => {
                    for (name, value) in parent.fields {
                        if name != "crossref" {
                            self.entries[at].fields.entry(name).or_insert(value);
                        }
                    }
                }
                None => self.warnings.push(format!(
                    "A bad cross reference---entry \"{}\" refers to entry \"{target}\", which doesn't exist",
                    self.entries[at].key
                )),
            }
            if !listed.iter().any(|k| k.eq_ignore_ascii_case(&target)) {
                self.entries[at].fields.remove("crossref");
            }
        }
    }

    fn state_of(&self, entry: &Entry) -> EntryState {
        let mut state = EntryState {
            key: entry.key.clone(),
            kind: entry.kind.to_ascii_lowercase(),
            ..EntryState::default()
        };
        for (name, value) in &entry.fields {
            let name = name.to_ascii_lowercase();
            if self.fields.contains(&name) {
                state.fields.insert(name, value.clone());
            }
        }
        for name in &self.entry_ints {
            state.ints.insert(name.clone(), 0);
        }
        for name in &self.entry_strs {
            state.strs.insert(name.clone(), String::new());
        }
        state
    }

    /// Run the style's commands, and hand back the `.bbl`.
    pub fn run(&mut self) -> String {
        for command in &self.style.commands {
            match command {
                Command::Execute(body) => {
                    self.current = None;
                    self.execute(Rc::new(body.clone()));
                }
                Command::Iterate(body) => {
                    for at in self.order.clone() {
                        self.current = Some(at);
                        self.execute(Rc::new(body.clone()));
                    }
                    self.current = None;
                }
                Command::Reverse(body) => {
                    for at in self.order.clone().into_iter().rev() {
                        self.current = Some(at);
                        self.execute(Rc::new(body.clone()));
                    }
                    self.current = None;
                }
                Command::Sort => {
                    // BibTeX sorts on the sort.key$ the style built, and the
                    // comparison is on characters, so a key a style forgot to
                    // lowercase sorts by ASCII -- which is why every style
                    // purifies and lowercases before sorting.
                    let keys: Vec<String> = self
                        .entries
                        .iter()
                        .map(|e| e.strs["sort.key$"].clone())
                        .collect();
                    self.order.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
                }
                _ => {}
            }
        }
        std::mem::take(&mut self.out.text)
    }

    /// Run one body.
    fn execute(&mut self, body: Rc<Vec<Token>>) {
        for token in body.iter() {
            match token {
                Token::Integer(i) => self.stack.push(Val::Int(*i)),
                Token::String(s) => self.stack.push(Val::Str(s.clone())),
                Token::Block(body) => self.stack.push(Val::Fun(Rc::new(body.clone()))),
                Token::Quoted(name) => match self.functions.get(name) {
                    Some(body) => self.stack.push(Val::Fun(body.clone())),
                    // A quoted name that is not a function is a variable being
                    // named for `:=`, which needs the name rather than its
                    // value.
                    None => self
                        .stack
                        .push(Val::Fun(Rc::new(vec![Token::Name(name.clone())]))),
                },
                Token::Name(name) => self.call(name),
            }
        }
    }

    /// Call a name: a builtin, a function, a variable, or a field.
    fn call(&mut self, name: &str) {
        if self.builtin(name) {
            return;
        }
        if let Some(body) = self.functions.get(name).cloned() {
            self.execute(body);
            return;
        }
        if let Some(value) = self.global_ints.get(name) {
            self.stack.push(Val::Int(*value));
            return;
        }
        if let Some(value) = self.global_strs.get(name) {
            self.stack.push(Val::Str(value.clone()));
            return;
        }
        if let Some(at) = self.current {
            let entry = &self.entries[at];
            if self.fields.contains(name) {
                self.stack.push(match entry.fields.get(name) {
                    Some(value) => Val::Str(value.clone()),
                    None => Val::Missing,
                });
                return;
            }
            if let Some(value) = entry.ints.get(name) {
                self.stack.push(Val::Int(*value));
                return;
            }
            if let Some(value) = entry.strs.get(name) {
                self.stack.push(Val::Str(value.clone()));
                return;
            }
        }
        // Outside an entry, an entry variable is still a name the style may
        // mention; BibTeX complains rather than stopping.
        self.warnings.push(format!("I couldn't execute {name}"));
    }

    fn pop(&mut self) -> Val {
        self.stack.pop().unwrap_or(Val::Missing)
    }

    fn pop_str(&mut self) -> String {
        self.pop().as_str()
    }

    fn pop_int(&mut self) -> i64 {
        self.pop().as_int().unwrap_or(0)
    }

    fn pop_fun(&mut self) -> Rc<Vec<Token>> {
        match self.pop() {
            Val::Fun(body) => body,
            _ => Rc::new(Vec::new()),
        }
    }

    /// Assign to whatever `Val::Fun` names, which is how `:=` works: the
    /// left-hand side arrives as a one-token function holding the name.
    fn assign(&mut self, target: &Rc<Vec<Token>>, value: Val) {
        let Some(Token::Name(name)) = target.first() else {
            return;
        };
        let name = name.clone();
        if let Some(slot) = self.global_ints.get_mut(&name) {
            *slot = value.as_int().unwrap_or(0);
            return;
        }
        if let Some(slot) = self.global_strs.get_mut(&name) {
            *slot = value.as_str();
            return;
        }
        if let Some(at) = self.current {
            if let Some(slot) = self.entries[at].ints.get_mut(&name) {
                *slot = value.as_int().unwrap_or(0);
                return;
            }
            if let Some(slot) = self.entries[at].strs.get_mut(&name) {
                *slot = value.as_str();
                return;
            }
        }
        self.warnings.push(format!("{name} is not a variable"));
    }

    /// The 37 builtins. Returns whether `name` was one.
    fn builtin(&mut self, name: &str) -> bool {
        match name {
            ">" | "<" => {
                let b = self.pop_int();
                let a = self.pop_int();
                let yes = match name {
                    ">" => a > b,
                    _ => a < b,
                };
                self.stack.push(Val::Int(yes as i64));
            }
            "=" => {
                let b = self.pop();
                let a = self.pop();
                let same = match (&a, &b) {
                    (Val::Int(a), Val::Int(b)) => a == b,
                    _ => a.as_str() == b.as_str(),
                };
                self.stack.push(Val::Int(same as i64));
            }
            "+" | "-" => {
                let b = self.pop_int();
                let a = self.pop_int();
                self.stack.push(Val::Int(match name {
                    "+" => a + b,
                    _ => a - b,
                }));
            }
            "*" => {
                let b = self.pop_str();
                let a = self.pop_str();
                self.stack.push(Val::Str(a + &b));
            }
            ":=" => {
                let target = self.pop_fun();
                let value = self.pop();
                self.assign(&target, value);
            }
            "add.period$" => {
                let s = self.pop_str();
                self.stack.push(Val::Str(add_period(&s)));
            }
            "call.type$" => {
                let kind = match self.current {
                    Some(at) => self.entries[at].kind.clone(),
                    None => String::new(),
                };
                let which = match self.functions.contains_key(&kind) {
                    true => kind,
                    false => "default.type".to_string(),
                };
                if let Some(body) = self.functions.get(&which).cloned() {
                    self.execute(body);
                }
            }
            "change.case$" => {
                let how = self.pop_str();
                let s = self.pop_str();
                self.stack.push(Val::Str(change_case(&s, &how)));
            }
            "chr.to.int$" => {
                let s = self.pop_str();
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => self.stack.push(Val::Int(c as i64)),
                    _ => {
                        self.warnings
                            .push(format!("\"{s}\" isn't a single character"));
                        self.stack.push(Val::Int(0));
                    }
                }
            }
            // The two limits BibTeX reports, which styles use as truncation
            // lengths -- `#1 entry.max$ substring$` is how a style keeps a
            // sort key inside what BibTeX can hold.
            "entry.max$" => self.stack.push(Val::Int(500)),
            "global.max$" => self.stack.push(Val::Int(200_000)),
            "cite$" => {
                let key = match self.current {
                    Some(at) => self.entries[at].key.clone(),
                    None => String::new(),
                };
                self.stack.push(Val::Str(key));
            }
            "duplicate$" => {
                let top = self.pop();
                self.stack.push(top.clone());
                self.stack.push(top);
            }
            "empty$" => {
                let top = self.pop();
                let empty = match &top {
                    Val::Missing => true,
                    other => other.as_str().trim().is_empty(),
                };
                self.stack.push(Val::Int(empty as i64));
            }
            "format.name$" => {
                let format = self.pop_str();
                let which = self.pop_int();
                let names = self.pop_str();
                self.stack
                    .push(Val::Str(format_name(&names, which, &format)));
            }
            "if$" => {
                let no = self.pop_fun();
                let yes = self.pop_fun();
                let test = self.pop_int();
                self.execute(match test > 0 {
                    true => yes,
                    false => no,
                });
            }
            "int.to.chr$" => {
                let i = self.pop_int();
                let c = u32::try_from(i)
                    .ok()
                    .and_then(char::from_u32)
                    .unwrap_or('\0');
                self.stack.push(Val::Str(c.to_string()));
            }
            "int.to.str$" => {
                let i = self.pop_int();
                self.stack.push(Val::Str(i.to_string()));
            }
            "missing$" => {
                let top = self.pop();
                self.stack
                    .push(Val::Int(matches!(top, Val::Missing) as i64));
            }
            "newline$" => self.out.newline(),
            "num.names$" => {
                let s = self.pop_str();
                self.stack.push(Val::Int(split_names(&s).len() as i64));
            }
            "pop$" => {
                self.pop();
            }
            "preamble$" => {
                let preamble = self.preamble.clone();
                self.stack.push(Val::Str(preamble));
            }
            "purify$" => {
                let s = self.pop_str();
                self.stack.push(Val::Str(purify(&s)));
            }
            "quote$" => self.stack.push(Val::Str("\"".into())),
            "skip$" => {}
            "stack$" => {
                let stack = std::mem::take(&mut self.stack);
                for value in stack.iter().rev() {
                    self.warnings.push(format!("{value:?}"));
                }
            }
            "substring$" => {
                let len = self.pop_int();
                let start = self.pop_int();
                let s = self.pop_str();
                self.stack.push(Val::Str(substring(&s, start, len)));
            }
            "swap$" => {
                let b = self.pop();
                let a = self.pop();
                self.stack.push(b);
                self.stack.push(a);
            }
            "text.length$" => {
                let s = self.pop_str();
                self.stack.push(Val::Int(text_length(&s) as i64));
            }
            "text.prefix$" => {
                let len = self.pop_int();
                let s = self.pop_str();
                self.stack.push(Val::Str(text_prefix(&s, len)));
            }
            "top$" => {
                let top = self.pop();
                self.warnings.push(format!("{top:?}"));
            }
            "type$" => {
                let kind = match self.current {
                    Some(at) => self.entries[at].kind.clone(),
                    None => String::new(),
                };
                // An entry whose type the style does not know reads as the
                // empty string, which is how a style tests for one.
                self.stack
                    .push(Val::Str(match self.functions.contains_key(&kind) {
                        true => kind,
                        false => String::new(),
                    }));
            }
            "warning$" => {
                let s = self.pop_str();
                self.warnings.push(s);
            }
            "while$" => {
                let body = self.pop_fun();
                let test = self.pop_fun();
                // A predicate that never falls is a style's bug, not a reason
                // to hang: BibTeX would spin, and this stops and says so.
                for _ in 0..1_000_000 {
                    self.execute(test.clone());
                    if self.pop_int() <= 0 {
                        return true;
                    }
                    self.execute(body.clone());
                }
                self.warnings.push("a while$ loop did not finish".into());
            }
            "width$" => {
                let s = self.pop_str();
                self.stack.push(Val::Int(width(&s)));
            }
            "write$" => {
                let s = self.pop_str();
                self.out.write(&s);
            }
            _ => return false,
        }
        true
    }
}

/// Run a style over a document's `.aux`, and hand back the `.bbl`.
///
/// This is what `bibtex FILE` does: read the `.aux` for the citations, the
/// style and the databases, read the databases, run the style, write the
/// `.bbl`.
pub fn run(aux: &Aux, style: &Style, db: &Bib) -> (String, Vec<String>) {
    let mut vm = Vm::new(style, db, aux);
    let bbl = vm.run();
    (bbl, std::mem::take(&mut vm.warnings))
}

// ---------------------------------------------------------------------------
// The string builtins, which is where the format's real behaviour lives.
// ---------------------------------------------------------------------------

/// Is the brace group starting at `at` a *special character* — a `{` whose
/// next character is a backslash? BibTeX treats one as a single character
/// whose insides no builtin touches.
fn is_special(chars: &[char], at: usize) -> bool {
    chars.get(at) == Some(&'{') && chars.get(at + 1) == Some(&'\\')
}

/// Where the control sequence beginning at `at` (the backslash) ends: TeX's
/// rule, which BibTeX follows -- a run of letters, or exactly one character
/// when the first is not a letter, so `\\'` is a control sequence and the `a`
/// after it is a letter to be measured, changed or purified.
fn cs_end(chars: &[char], at: usize) -> usize {
    let mut i = at + 1;
    match chars.get(i) {
        Some(c) if c.is_ascii_alphabetic() => {
            while chars.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
                i += 1;
            }
        }
        Some(_) => i += 1,
        None => {}
    }
    i
}

/// Where the brace group opened at `at` ends (the index of its `}`), or the
/// end of the string when it is never closed.
fn group_end(chars: &[char], at: usize) -> usize {
    let mut depth = 0usize;
    let mut i = at;
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

/// `add.period$`: a period unless the string already ends in one, in `!` or in
/// `?` — looking past any closing braces, so `{Who?}` is already punctuated.
fn add_period(s: &str) -> String {
    for c in s.chars().rev() {
        match c {
            '}' => continue,
            '.' | '!' | '?' => return s.to_string(),
            _ => break,
        }
    }
    format!("{s}.")
}

/// `text.length$`: characters, where a brace is not one and a special
/// character is exactly one.
fn text_length(s: &str) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut n = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        if is_special(&chars, i) {
            i = group_end(&chars, i) + 1;
            n += 1;
            continue;
        }
        if chars[i] != '{' && chars[i] != '}' {
            n += 1;
        }
        i += 1;
    }
    n
}

/// `text.prefix$`: the first `len` characters by that same counting, with any
/// brace groups it cut through closed again.
fn text_prefix(s: &str, len: i64) -> String {
    if len <= 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut n = 0i64;
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < chars.len() && n < len {
        if is_special(&chars, i) {
            let end = group_end(&chars, i);
            out.extend(&chars[i..=end.min(chars.len() - 1)]);
            i = end + 1;
            n += 1;
            continue;
        }
        match chars[i] {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => n += 1,
        }
        out.push(chars[i]);
        i += 1;
    }
    for _ in 0..depth {
        out.push('}');
    }
    out
}

/// `substring$`: BibTeX counts characters from 1, and a negative start counts
/// back from the end, with the substring ending there.
fn substring(s: &str, start: i64, len: i64) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    if len <= 0 || start == 0 {
        return String::new();
    }
    let (from, to) = match start > 0 {
        true => (start - 1, start - 1 + len),
        // -2 3 means the three characters ending two from the end.
        false => {
            let end = n + start + 1;
            (end - len, end)
        }
    };
    let from = from.clamp(0, n) as usize;
    let to = to.clamp(0, n) as usize;
    chars[from..to.max(from)].iter().collect()
}

/// `purify$`: what is left of a string when everything but letters, digits and
/// single spaces is taken out. A special character keeps its letters and loses
/// its control sequence, which is how an accented name sorts beside a plain
/// one.
fn purify(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        if is_special(&chars, i) {
            let end = group_end(&chars, i);
            let mut j = i + 1;
            while j < end {
                if chars[j] == '\\' {
                    // Skip the control sequence's name: it is markup, not text.
                    j = cs_end(&chars, j).min(end);
                    continue;
                }
                if chars[j].is_alphanumeric() {
                    out.push(chars[j]);
                }
                j += 1;
            }
            i = end + 1;
            continue;
        }
        match chars[i] {
            c if c.is_alphanumeric() => out.push(c),
            ' ' | '-' | '~' => out.push(' '),
            _ => {}
        }
        i += 1;
    }
    out
}

/// `change.case$`: `t` for title case (everything but the first character
/// lowercased), `l` for lower, `u` for upper. Braces protect what is inside
/// them, and a special character's control sequence is left alone while the
/// letters after it are changed.
fn change_case(s: &str, how: &str) -> String {
    let how = how.chars().next().unwrap_or('t').to_ascii_lowercase();
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    // Title case leaves the first character alone, and the character after a
    // colon-and-space, as BibTeX does.
    let mut at_start = true;
    while i < chars.len() {
        if is_special(&chars, i) && depth == 0 {
            let end = group_end(&chars, i);
            out.push('{');
            let mut j = i + 1;
            while j < end {
                if chars[j] == '\\' {
                    // A control sequence is markup: it comes through as it was.
                    let stop = cs_end(&chars, j).min(end);
                    out.extend(&chars[j..stop]);
                    j = stop;
                    continue;
                }
                out.push(convert(chars[j], how, at_start));
                j += 1;
            }
            out.push('}');
            i = end + 1;
            at_start = false;
            continue;
        }
        match chars[i] {
            '{' => {
                depth += 1;
                out.push('{');
            }
            '}' => {
                depth = depth.saturating_sub(1);
                out.push('}');
            }
            c if depth > 0 => out.push(c),
            c => {
                out.push(convert(c, how, at_start));
                if !c.is_whitespace() {
                    at_start = c == ':';
                }
            }
        }
        if i == 0 {
            at_start = false;
        }
        i += 1;
    }
    out
}

fn convert(c: char, how: char, protected: bool) -> char {
    match how {
        'u' => c.to_ascii_uppercase(),
        'l' => c.to_ascii_lowercase(),
        // Title case: the first character stays as it was.
        _ if protected => c,
        _ => c.to_ascii_lowercase(),
    }
}

/// The names in a `author`/`editor` field: split on ` and ` at brace level 0.
fn split_names(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        // " and " with a space on each side, outside braces.
        if depth == 0
            && chars[i].is_whitespace()
            && chars
                .get(i + 1..i + 4)
                .map(|w| w.iter().collect::<String>().eq_ignore_ascii_case("and"))
                == Some(true)
            && chars.get(i + 4).is_some_and(|c| c.is_whitespace())
        {
            out.push(current.trim().to_string());
            current.clear();
            i += 5;
            continue;
        }
        current.push(chars[i]);
        i += 1;
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

/// One name, split the way `bibtex.web` §386-§399 splits it.
#[derive(Debug, Default, PartialEq)]
struct Name {
    first: Vec<String>,
    von: Vec<String>,
    last: Vec<String>,
    jr: Vec<String>,
}

/// The first letter of a token at brace level 0 — what decides whether a token
/// is part of the von: `von`, `de la`, `van der` are lowercase, `Van` is not.
/// A special character's control sequence is skipped, so `{\'e}cole` is
/// lowercase.
fn first_letter(token: &str) -> Option<char> {
    let chars: Vec<char> = token.chars().collect();
    let mut i = 0usize;
    let mut depth = 0usize;
    while i < chars.len() {
        match chars[i] {
            '{' => {
                if chars.get(i + 1) == Some(&'\\') {
                    // Inside a special character, the first letter that is not
                    // part of a control sequence counts.
                    let end = group_end(&chars, i);
                    let mut j = i + 1;
                    while j < end {
                        if chars[j] == '\\' {
                            j = cs_end(&chars, j).min(end);
                            continue;
                        }
                        if chars[j].is_alphabetic() {
                            return Some(chars[j]);
                        }
                        j += 1;
                    }
                    i = end + 1;
                    continue;
                }
                depth += 1;
            }
            '}' => depth = depth.saturating_sub(1),
            c if depth == 0 && c.is_alphabetic() => return Some(c),
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_von(token: &str) -> bool {
    first_letter(token).is_some_and(|c| c.is_lowercase())
}

/// Split `name` into its four parts.
fn parse_name(name: &str) -> Name {
    // The commas that separate the three forms are at brace level 0.
    let chars: Vec<char> = name.chars().collect();
    let mut parts: Vec<String> = vec![String::new()];
    let mut depth = 0usize;
    for &c in &chars {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(String::new());
                continue;
            }
            _ => {}
        }
        parts.last_mut().expect("a part").push(c);
    }
    // Tokens are separated by whitespace at brace level 0: the space inside
    // `Erd{\\H o}s` is part of the accent, not a break between two names.
    let tokens = |s: &str| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut depth = 0usize;
        for c in s.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                c if c.is_whitespace() && depth == 0 => {
                    if !current.is_empty() {
                        out.push(std::mem::take(&mut current));
                    }
                    continue;
                }
                _ => {}
            }
            current.push(c);
        }
        if !current.is_empty() {
            out.push(current);
        }
        out
    };

    let mut out = Name::default();
    match parts.len() {
        // "First von Last"
        1 => {
            let all = tokens(&parts[0]);
            let (mut von_start, mut von_end) = (all.len(), all.len());
            for (i, token) in all.iter().enumerate() {
                // The last token is always part of Last, even when lowercase.
                if i + 1 < all.len() && is_von(token) {
                    if von_start == all.len() {
                        von_start = i;
                    }
                    von_end = i + 1;
                }
            }
            match von_start < all.len() {
                true => {
                    out.first = all[..von_start].to_vec();
                    out.von = all[von_start..von_end].to_vec();
                    out.last = all[von_end..].to_vec();
                }
                false => {
                    let split = all.len().saturating_sub(1);
                    out.first = all[..split].to_vec();
                    out.last = all[split..].to_vec();
                }
            }
        }
        // "von Last, First" and "von Last, Jr, First"
        _ => {
            let head = tokens(&parts[0]);
            let mut von_end = 0usize;
            for (i, token) in head.iter().enumerate() {
                if i + 1 < head.len() && is_von(token) {
                    von_end = i + 1;
                }
            }
            out.von = head[..von_end].to_vec();
            out.last = head[von_end..].to_vec();
            match parts.len() {
                2 => out.first = tokens(&parts[1]),
                _ => {
                    out.jr = tokens(&parts[1]);
                    out.first = tokens(&parts[2]);
                }
            }
        }
    }
    out
}

/// A token abbreviated: its first letter, with `after` following each piece --
/// the text the format put after its letter, which is where the period in
/// `{f.}` comes from. A special character stays whole (`{\\'E}mile` abbreviates
/// to `{\\'E}.`), and a hyphenated token abbreviates piece by piece, so
/// `Jean-Paul` is `J.-P.`.
fn abbreviate(token: &str, after: &str) -> String {
    token
        .split('-')
        .filter_map(|piece| {
            let chars: Vec<char> = piece.chars().collect();
            if is_special(&chars, 0) {
                let end = group_end(&chars, 0);
                let head: String = chars[..=end.min(chars.len() - 1)].iter().collect();
                return Some(format!("{head}{after}"));
            }
            first_letter(piece).map(|c| format!("{c}{after}"))
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// The tokens of one name part, joined the way BibTeX joins them: a tie before
/// the last token, a tie after a first token short enough to leave a line
/// ending on an initial, and a space everywhere else. This is what writes
/// `Donald~E. Knuth`, `J.~B. P.~M. Lamarck` and `Maria de~la Vega`.
fn join_tokens(pieces: &[String]) -> String {
    let mut out = String::new();
    for (i, piece) in pieces.iter().enumerate() {
        if i > 0 {
            let last = i + 1 == pieces.len();
            let after_short_first = i == 1 && text_length(&pieces[0]) < 3;
            out.push(match last || after_short_first {
                true => '~',
                false => ' ',
            });
        }
        out.push_str(piece);
    }
    out
}

/// `format.name$`: the `which`th name of `names`, rendered through `format`.
fn format_name(names: &str, which: i64, format: &str) -> String {
    let all = split_names(names);
    let Some(name) = all.get((which.max(1) - 1) as usize) else {
        return String::new();
    };
    let name = parse_name(name);
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '{' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let end = group_end(&chars, i);
        let group: String = chars[i + 1..end.min(chars.len())].iter().collect();
        out.push_str(&render_group(&group, &name));
        i = end + 1;
    }
    out
}

/// One `{...}` of a format string: text, a letter saying which part of the
/// name, and text. A doubled letter is the part in full; a single one
/// abbreviates it. If the part is empty the whole group disappears, which is
/// what makes `{, jj}` cost nothing for a name with no Jr.
fn render_group(group: &str, name: &Name) -> String {
    let chars: Vec<char> = group.chars().collect();
    let Some(at) = chars.iter().position(|c| "fvljFVLJ".contains(*c)) else {
        // A group with no letter is literal text.
        return group.to_string();
    };
    let letter = chars[at].to_ascii_lowercase();
    let long = chars.get(at + 1).map(|c| c.to_ascii_lowercase()) == Some(letter);
    let mut after = at + 1 + long as usize;

    let pre: String = chars[..at].iter().collect();
    // An explicit separator, as in `{ll{ }}`.
    let mut separator: Option<String> = None;
    if chars.get(after) == Some(&'{') {
        let end = group_end(&chars, after);
        separator = Some(chars[after + 1..end.min(chars.len())].iter().collect());
        after = end + 1;
    }
    let post: String = chars[after.min(chars.len())..].iter().collect();

    let tokens = match letter {
        'f' => &name.first,
        'v' => &name.von,
        'l' => &name.last,
        _ => &name.jr,
    };
    if tokens.is_empty() {
        return String::new();
    }

    // A tie at the very end of a group is discretionary: BibTeX writes `~`
    // when what comes before it is short -- an initial, `de` -- and a space
    // otherwise, so a line never breaks after `J.` but may after a surname.
    // `~~` is a tie that is not up for discussion.
    let mut post = post;
    let mut ending: Option<char> = None;
    if post.ends_with("~~") {
        post.pop();
    } else if post.ends_with('~') {
        post.pop();
        ending = Some('~');
    }

    // What is left of the post-text follows every token, which is how `{f.}`
    // puts a period after each initial.
    let pieces: Vec<String> = match long {
        true => tokens.iter().map(|t| format!("{t}{post}")).collect(),
        false => tokens.iter().map(|t| abbreviate(t, &post)).collect(),
    };
    let joined = match &separator {
        // A format may say what goes between the tokens, as alpha's `{v{}}`
        // does to run initials together.
        Some(sep) => pieces.join(sep),
        None => join_tokens(&pieces),
    };

    let mut body = format!("{pre}{joined}");
    if ending == Some('~') {
        body.push(match text_length(&body) < 3 {
            true => '~',
            false => ' ',
        });
    }
    body
}

/// `width$`: the string's width in cmr10, in thousandths of the design size,
/// from the table `bibtex.web` §  carries. Braces count; a special character
/// contributes the width of the symbol its control sequence names, and nothing
/// for a control sequence that names no symbol.
fn width(s: &str) -> i64 {
    let chars: Vec<char> = s.chars().collect();
    let mut total = 0i64;
    let mut i = 0usize;
    while i < chars.len() {
        if is_special(&chars, i) {
            let end = group_end(&chars, i);
            let mut j = i + 1;
            while j < end {
                if chars[j] == '\\' {
                    let start = j + 1;
                    j = cs_end(&chars, j).min(end);
                    let cs: String = chars[start..j].iter().collect();
                    total += symbol_width(&cs);
                    // The space that ends a control word is part of the name,
                    // as it is in TeX: `{\\relax x}` is as wide as an x.
                    if cs.chars().all(|c| c.is_ascii_alphabetic()) {
                        while j < end && chars[j].is_whitespace() {
                            j += 1;
                        }
                    }
                    continue;
                }
                if chars[j] != '{' && chars[j] != '}' {
                    total += char_width(chars[j]);
                }
                j += 1;
            }
            i = end + 1;
            continue;
        }
        total += char_width(chars[i]);
        i += 1;
    }
    total
}

/// The width of a symbol a control sequence names, for the thirteen BibTeX
/// knows. Anything else is nothing, as an accent is.
fn symbol_width(cs: &str) -> i64 {
    match cs {
        "aa" => 500,
        "AA" => 750,
        "ae" => 722,
        "AE" => 903,
        "o" => 500,
        "O" => 778,
        "oe" => 778,
        "OE" => 1014,
        "ss" => 500,
        "l" => 278,
        "L" => 625,
        "i" => 278,
        "j" => 306,
        _ => 0,
    }
}

/// cmr10's widths, in thousandths, as `bibtex.web` tabulates them.
fn char_width(c: char) -> i64 {
    const WIDTHS: [i64; 96] = [
        278, 278, 500, 833, 500, 833, 778, 278, 389, 389, 500, 778, 278, 333, 278, 500, // 32
        500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 278, 278, 278, 778, 472, 472, // 48
        778, 750, 708, 722, 764, 681, 653, 785, 750, 361, 514, 778, 625, 917, 750, 778, // 64
        681, 778, 736, 556, 722, 750, 750, 1028, 750, 750, 611, 278, 500, 278, 500, 278, // 80
        278, 500, 556, 444, 556, 444, 306, 500, 556, 278, 306, 528, 278, 833, 556, 500, // 96
        556, 528, 392, 394, 389, 556, 528, 722, 528, 528, 444, 500, 1000, 500, 500, 0, // 112
    ];
    match c as usize {
        code @ 32..=127 => WIDTHS[code - 32],
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four builtins that carry the format's behaviour, on the cases that
    /// tell a right implementation from a plausible one. Every expected value
    /// here was taken from the real `bibtex` -- a style that prints it, run,
    /// and its `.bbl` read -- rather than from reasoning about the format.
    #[test]
    fn the_string_builtins_do_what_bibtex_does() {
        // A brace group beginning with a backslash is one character, however
        // long it is, and braces themselves are not characters at all.
        assert_eq!(text_length("{\\ss}"), 1);
        assert_eq!(text_length("{\\'a}bc"), 3);
        assert_eq!(text_length("{ab}c"), 3);
        assert_eq!(text_prefix("{\\'a}bcd", 3), "{\\'a}bc");
        // A prefix that cuts through a brace group closes it again, and a
        // group already closed stays as it was -- both from bibtex itself.
        assert_eq!(text_prefix("{ab}cd", 3), "{ab}c");
        assert_eq!(text_prefix("{ab}cd", 1), "{a}");
        assert_eq!(text_prefix("a{bc}d", 2), "a{b}");
        assert_eq!(text_prefix("ab{cd", 3), "ab{c}");
        assert_eq!(text_prefix("{\\'a}bcd", 2), "{\\'a}b");

        // Case changing leaves braced text and control sequences alone.
        assert_eq!(change_case("aBc {\\'e} DEF", "t"), "abc {\\'e} def");
        assert_eq!(change_case("aBc {\\'e} DEF", "u"), "ABC {\\'E} DEF");
        assert_eq!(change_case("The {TeX}book", "l"), "the {TeX}book");

        // purify$ keeps letters and digits, drops markup, and turns what
        // separates words into single spaces -- which is what a sort key is
        // built from.
        assert_eq!(purify("a{\\'e}B{XY}c"), "aeBXYc");
        assert_eq!(purify("Knuth, Donald E."), "Knuth Donald E");

        // substring$ counts from one, and a negative start counts back from
        // the end with the substring ending there.
        assert_eq!(substring("abcdef", 2, 3), "bcd");
        assert_eq!(substring("abcdef", -2, 3), "cde");
        assert_eq!(substring("abcdef", 1, 100), "abcdef");
        assert_eq!(substring("abcdef", 0, 3), "");

        // add.period$ looks past closing braces for the punctuation.
        assert_eq!(add_period("A title"), "A title.");
        assert_eq!(add_period("Who?"), "Who?");
        assert_eq!(add_period("{Who?}"), "{Who?}");
        assert_eq!(add_period("A title!"), "A title!");
    }

    /// `width$` measures in cmr10, and a special character is worth the symbol
    /// its control sequence names -- 500 for `{\ss}`, nothing for an accent.
    #[test]
    fn width_measures_what_bibtex_measures() {
        assert_eq!(width("A"), 750);
        assert_eq!(width("AV"), 1500, "no kerning: BibTeX adds widths");
        assert_eq!(width("ff"), 612, "and no ligature either");
        assert_eq!(width(" "), 278);
        assert_eq!(width("{ab}"), 2056, "a brace is 500 wide");
        assert_eq!(width("{\\ss}"), 500);
        assert_eq!(width("{\\ss}x"), 1028);
        assert_eq!(width("{\\ae}"), 722);
        assert_eq!(
            width("{\\'a}"),
            500,
            "an accent is worth nothing, the a 500"
        );
        assert_eq!(width("{\\relax x}"), 528, "an unknown control sequence too");
        assert_eq!(width("{\\ }"), 0);
    }

    /// Splitting a name is where a reimplementation goes wrong, so each of the
    /// three forms BibTeX accepts is checked, with the von and the junior.
    #[test]
    fn a_name_is_split_the_way_bibtex_splits_it() {
        let name = parse_name("Donald E. Knuth");
        assert_eq!(name.first, ["Donald", "E."]);
        assert_eq!(name.last, ["Knuth"]);

        // A lowercase token is the von, and the last token is never part of it.
        let name = parse_name("Ludwig van Beethoven");
        assert_eq!(name.first, ["Ludwig"]);
        assert_eq!(name.von, ["van"]);
        assert_eq!(name.last, ["Beethoven"]);

        let name = parse_name("de la Vega, Maria");
        assert_eq!(name.von, ["de", "la"]);
        assert_eq!(name.last, ["Vega"]);
        assert_eq!(name.first, ["Maria"]);

        let name = parse_name("King, Jr., Martin Luther");
        assert_eq!(name.last, ["King"]);
        assert_eq!(name.jr, ["Jr."]);
        assert_eq!(name.first, ["Martin", "Luther"]);

        // The space inside an accent does not break a token, and the accent's
        // letter is what decides the case.
        let name = parse_name("Erd{\\H o}s, P.");
        assert_eq!(name.last, ["Erd{\\H o}s"]);
        assert_eq!(name.first, ["P."]);

        // A braced name is one token, `and` inside it and all.
        assert_eq!(split_names("{Barnes and Noble, Inc.}").len(), 1);
        assert_eq!(split_names("A. One and B. Two and C. Three").len(), 3);
    }

    /// `format.name$` against the real `bibtex`: every one of these was printed
    /// by it. The ties are the part worth pinning -- BibTeX writes one before
    /// the last token of a part and after a first token too short to end a
    /// line on, and the tie a format ends with is discretionary.
    #[test]
    fn format_name_writes_what_bibtex_writes() {
        for (names, format, want) in [
            ("Donald E. Knuth", "{ff~}{vv~}{ll}{, jj}", "Donald~E. Knuth"),
            ("Donald E. Knuth", "{f.~}{vv~}{ll}{, jj}", "D.~E. Knuth"),
            (
                "Ludwig van Beethoven",
                "{vv~}{ll}{, jj}{, ff}",
                "van Beethoven, Ludwig",
            ),
            (
                "de la Vega, Maria",
                "{ff~}{vv~}{ll}{, jj}",
                "Maria de~la Vega",
            ),
            (
                "King, Jr., Martin Luther",
                "{ff~}{vv~}{ll}{, jj}",
                "Martin~Luther King, Jr.",
            ),
            (
                "King, Jr., Martin Luther",
                "{vv~}{ll}{, jj}{, f.}",
                "King, Jr., M.~L.",
            ),
            ("Al Bo", "{ff~}{vv~}{ll}{, jj}", "Al~Bo"),
            ("Erd{\\H o}s, P.", "{ff~}{vv~}{ll}{, jj}", "P.~Erd{\\H o}s"),
            ("Jean-Paul Sartre", "{f.~}{vv~}{ll}{, jj}", "J.-P. Sartre"),
            (
                "Jean Baptiste Pierre Lamarck",
                "{ff~}{vv~}{ll}{, jj}",
                "Jean Baptiste~Pierre Lamarck",
            ),
            (
                "Jean Baptiste Pierre Marie Lamarck",
                "{f.~}{ll}",
                "J.~B. P.~M. Lamarck",
            ),
            (
                "van de la Vega Lopez, Maria",
                "{vv~}{ll}",
                "van de~la Vega~Lopez",
            ),
            // alpha's label format: initials run together with nothing between.
            ("Aho, Alfred V.", "{v{}}{l{}}", "A"),
            ("de la Vega, Maria", "{v{}}{l{}}", "dlV"),
            // A name the style has no room for still comes back whole.
            (
                "{Barnes and Noble, Inc.}",
                "{ff~}{vv~}{ll}{, jj}",
                "{Barnes and Noble, Inc.}",
            ),
        ] {
            assert_eq!(
                format_name(names, 1, format),
                want,
                "{names:?} through {format:?}"
            );
        }
        // The second of three, and asking past the end is empty rather than a
        // panic.
        assert_eq!(format_name("A. One and B. Two", 2, "{ll}"), "Two");
        assert_eq!(format_name("A. One", 7, "{ll}"), "");
    }

    /// The `.bbl` is broken at 79 columns with a two-space continuation, which
    /// is why a bibliography looks the way it does.
    #[test]
    fn a_long_line_is_broken_where_bibtex_breaks_it() {
        let mut out = Out::default();
        let words: Vec<String> = (1..30).map(|i| format!("word{i:02}")).collect();
        out.write(&words.join(" "));
        out.newline();
        let lines: Vec<&str> = out.text.lines().collect();
        assert_eq!(lines[0].chars().count(), 76);
        assert!(lines[1].starts_with("  word12"), "{:?}", lines[1]);
        assert!(lines.iter().all(|l| l.chars().count() <= 79));

        // A run with nothing to break at goes out whole rather than being cut
        // in the middle of a word.
        let mut out = Out::default();
        out.write(&"x".repeat(200));
        out.newline();
        assert_eq!(out.text.trim_end().chars().count(), 200);

        // Trailing whitespace is dropped, leading whitespace is not.
        let mut out = Out::default();
        out.write("  trailing   ");
        out.newline();
        assert_eq!(out.text, "  trailing\n");
    }
}
