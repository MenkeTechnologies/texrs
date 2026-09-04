//! Cutting a `tikzpicture` body into the commands it is made of.
//!
//! A picture is not a list of `\draw`s: it has `scope` environments that hand
//! their options down to everything inside them (§12.3.1) and `\foreach` loops
//! that write the same path several times over with a different number in it
//! (§88). Reading only the top-level `\draw`s of a picture built out of those
//! draws a fraction of it.
//!
//! Everything here is textual. A `\foreach` is expanded by substituting its
//! variable and re-scanning, which is what TeX does with it too, and means a
//! loop nested inside a loop needs no extra machinery.

/// What the path is for: `\draw`, `\fill`, `\filldraw`, `\path`, `\clip`,
/// `\shade` or `\shadedraw` (§15.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `\path` -- the path is built and nothing is painted.
    None,
    /// `\draw`, which is `\path[draw]`.
    Draw,
    /// `\fill`, which is `\path[fill]`.
    Fill,
    /// `\filldraw`, which is `\path[fill,draw]`.
    FillDraw,
    /// `\clip`, which is `\path[clip]`.
    Clip,
    /// `\shade` and `\shadedraw`. The path is read; no shading is painted.
    Shade,
    /// A standalone `\node` or `\coordinate`, which builds no path at all.
    Node,
}

impl Action {
    /// The action a command name asks for.
    fn named(name: &str) -> Option<Action> {
        match name {
            "path" => Some(Action::None),
            "draw" => Some(Action::Draw),
            "fill" => Some(Action::Fill),
            "filldraw" => Some(Action::FillDraw),
            "clip" => Some(Action::Clip),
            "shade" | "shadedraw" => Some(Action::Shade),
            "node" | "coordinate" => Some(Action::Node),
            _ => None,
        }
    }
}

/// One thing found in a picture body.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    /// A path command: what it is for, its `[...]`, and everything up to `;`.
    Path {
        action: Action,
        options: String,
        body: String,
    },
    /// A `scope`, with the options it hands down and the body it encloses.
    Scope { options: String, body: String },
}

/// The commands in a picture body, in order.
pub fn commands(body: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find('\\') {
        let (before, after) = rest.split_at(at);
        // A `%` comments out the rest of its line, backslash and all.
        if let Some(comment) = last_comment(before) {
            rest = match rest[comment..].find('\n') {
                Some(newline) => &rest[comment + newline + 1..],
                None => "",
            };
            continue;
        }
        let name: String = after[1..]
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        let tail = &after[1 + name.len()..];
        match name.as_str() {
            "begin" => match environment(tail) {
                Some(("scope", options, inner, after)) => {
                    out.push(Chunk::Scope {
                        options,
                        body: inner,
                    });
                    rest = after;
                }
                // A `tikzpicture` nested inside one, or any other environment,
                // is read straight through: its contents are still commands.
                _ => rest = tail,
            },
            "foreach" => {
                let (expanded, after) = foreach(tail);
                out.extend(commands(&expanded));
                rest = after;
            }
            _ => match Action::named(&name) {
                Some(action) => {
                    let (options, text, after) = statement(tail);
                    // A standalone `\node` is spelled the same as the `node`
                    // operator a path carries, so it is handed to the path
                    // reader with its word and its options put back in front
                    // rather than given a second parser of its own.
                    let (options, body) = match action {
                        Action::Node => (String::new(), format!("{name}[{options}] {text}")),
                        _ => (options, text),
                    };
                    out.push(Chunk::Path {
                        action,
                        options,
                        body,
                    });
                    rest = after;
                }
                None => rest = tail,
            },
        }
    }
    out
}

/// Where a `%` starts on the line the next backslash sits on.
///
/// A `%` comments out the rest of its line, so a `\draw` after one on the same
/// line is not a command -- reading it as one draws a path the document took
/// out.
fn last_comment(text: &str) -> Option<usize> {
    let line = text.rfind('\n').map(|at| at + 1).unwrap_or(0);
    text[line..].find('%').map(|at| line + at)
}

/// `[options] body ;` -- one path command's text.
fn statement(text: &str) -> (String, String, &str) {
    let (options, rest) = bracketed(text);
    let mut depth = 0i32;
    for (at, ch) in rest.char_indices() {
        match ch {
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            ';' if depth == 0 => {
                return (options, rest[..at].to_string(), &rest[at + 1..]);
            }
            _ => {}
        }
    }
    // A command with no semicolon runs to the end of the picture, which is
    // what a truncated body leaves behind. Reading it is better than dropping
    // it: the operators it names are still the ones the document wrote.
    (options, rest.to_string(), "")
}

/// `{name}[options] ... \end{name}`, with nesting counted.
fn environment(text: &str) -> Option<(&str, String, String, &str)> {
    let open = text.trim_start().strip_prefix('{')?;
    let close = open.find('}')?;
    let name = &open[..close];
    let (options, body) = bracketed(&open[close + 1..]);
    let opening = format!("\\begin{{{name}}}");
    let closing = format!("\\end{{{name}}}");
    let mut depth = 1usize;
    let mut at = 0usize;
    while at < body.len() {
        let next_open = body[at..].find(&opening).map(|i| at + i);
        let next_close = body[at..].find(&closing).map(|i| at + i);
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                at = o + opening.len();
            }
            (_, Some(c)) => {
                depth -= 1;
                if depth == 0 {
                    return Some((
                        name,
                        options,
                        body[..c].to_string(),
                        &body[c + closing.len()..],
                    ));
                }
                at = c + closing.len();
            }
            _ => break,
        }
    }
    None
}

/// `\foreach \x in {list} body` written out once per value (§88).
fn foreach(text: &str) -> (String, &str) {
    let mut rest = text.trim_start();
    let mut variables = Vec::new();
    while let Some(after) = rest.strip_prefix('\\') {
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        if name.is_empty() {
            break;
        }
        rest = after[name.len()..].trim_start();
        variables.push(name);
        match rest.strip_prefix('/') {
            Some(after) => rest = after.trim_start(),
            None => break,
        }
    }
    // `\foreach \x [count=\i] in ...` -- the options are read past, not used.
    let (_, rest) = bracketed(rest);
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix("in") else {
        return (String::new(), rest);
    };
    let (list, rest) = braced(rest.trim_start());
    let rest = rest.trim_start();
    let (body, after) = match rest.starts_with('{') {
        true => braced(rest),
        // A loop over a single command runs to that command's semicolon.
        false => match rest.find(';') {
            Some(at) => (rest[..=at].to_string(), &rest[at + 1..]),
            None => (rest.to_string(), ""),
        },
    };
    let mut out = String::new();
    for entry in values(&list) {
        let mut once = body.clone();
        for (variable, value) in variables.iter().zip(entry.split('/')) {
            once = substitute(&once, variable, value.trim());
        }
        out.push_str(&once);
        out.push('\n');
    }
    (out, after)
}

/// The values a `\foreach` list names, with `1,...,5` written out.
fn values(list: &str) -> Vec<String> {
    let entries: Vec<&str> = split_commas(list);
    let mut out: Vec<String> = Vec::new();
    for (at, entry) in entries.iter().enumerate() {
        let entry = entry.trim();
        if entry != "..." {
            out.push(entry.to_string());
            continue;
        }
        // `a,b,...,z` steps by b-a; `a,...,z` steps by one.
        let (Some(last), Some(next)) = (out.last(), entries.get(at + 1)) else {
            continue;
        };
        let (Ok(from), Ok(to)) = (last.parse::<f64>(), next.trim().parse::<f64>()) else {
            continue;
        };
        let step = match out.len() >= 2 {
            true => match out[out.len() - 2].parse::<f64>() {
                Ok(before) => from - before,
                Err(_) => 1.0,
            },
            false => 1.0,
        };
        let step = match step == 0.0 {
            true => 1.0,
            false => step,
        };
        let mut value = from + step;
        while (step > 0.0 && value < to) || (step < 0.0 && value > to) {
            out.push(format(value));
            value += step;
        }
    }
    out
}

/// A loop value without the trailing zeros a float prints with.
fn format(value: f64) -> String {
    match value.fract() == 0.0 {
        true => format!("{}", value as i64),
        false => format!("{value}"),
    }
}

/// Put `value` where `\variable` stands, and nowhere else.
///
/// `\x` must not be found inside `\xshift`, which is a real option and would
/// come out as `0shift` if the name were matched by prefix alone.
fn substitute(text: &str, variable: &str, value: &str) -> String {
    let needle = format!("\\{variable}");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        let after = &rest[at + needle.len()..];
        out.push_str(&rest[..at]);
        match after.chars().next() {
            Some(c) if c.is_ascii_alphabetic() => out.push_str(&needle),
            _ => out.push_str(value),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The commas of a list that are not inside braces or parentheses.
fn split_commas(list: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (at, ch) in list.char_indices() {
        match ch {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&list[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    out.push(&list[start..]);
    out.into_iter().filter(|e| !e.trim().is_empty()).collect()
}

/// `[...]` at the front, brace-counted, and what follows.
fn bracketed(text: &str) -> (String, &str) {
    let text = text.trim_start();
    let Some(open) = text.strip_prefix('[') else {
        return (String::new(), text);
    };
    let mut depth = 0i32;
    for (at, ch) in open.char_indices() {
        match ch {
            '[' | '{' => depth += 1,
            '}' => depth -= 1,
            ']' if depth == 0 => return (open[..at].to_string(), &open[at + 1..]),
            ']' => depth -= 1,
            _ => {}
        }
    }
    (String::new(), text)
}

/// `{...}` at the front, brace-counted, and what follows.
fn braced(text: &str) -> (String, &str) {
    let Some(open) = text.strip_prefix('{') else {
        return (String::new(), text);
    };
    let mut depth = 0i32;
    for (at, ch) in open.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' if depth == 0 => return (open[..at].to_string(), &open[at + 1..]),
            '}' => depth -= 1,
            _ => {}
        }
    }
    (String::new(), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_command_is_read_with_the_action_it_names() {
        let chunks = commands(r"\draw (0,0) -- (1,1); \fill (0,0) circle[radius=1]; \path (2,2);");
        assert_eq!(chunks.len(), 3);
        match &chunks[0] {
            Chunk::Path { action, body, .. } => {
                assert_eq!(*action, Action::Draw);
                assert_eq!(body.trim(), "(0,0) -- (1,1)");
            }
            other => panic!("a path, not {other:?}"),
        }
        assert!(matches!(
            chunks[1],
            Chunk::Path {
                action: Action::Fill,
                ..
            }
        ));
    }

    #[test]
    fn a_scope_keeps_its_body_and_its_options() {
        let chunks = commands(
            r"\begin{scope}[xshift=1cm]\draw (0,0) -- (1,0);\end{scope}\draw (2,2) -- (3,3);",
        );
        assert_eq!(chunks.len(), 2);
        match &chunks[0] {
            Chunk::Scope { options, body } => {
                assert_eq!(options, "xshift=1cm");
                assert!(body.contains("(1,0)"));
            }
            other => panic!("a scope, not {other:?}"),
        }
    }

    #[test]
    fn a_loop_writes_its_body_once_per_value() {
        let chunks = commands(r"\foreach \x in {0,1,2} { \draw (\x,0) -- (\x,1); }");
        assert_eq!(chunks.len(), 3);
        let bodies: Vec<String> = chunks
            .iter()
            .map(|c| match c {
                Chunk::Path { body, .. } => body.trim().to_string(),
                other => panic!("a path, not {other:?}"),
            })
            .collect();
        assert_eq!(bodies[0], "(0,0) -- (0,1)");
        assert_eq!(bodies[2], "(2,0) -- (2,1)");
    }

    #[test]
    fn a_range_is_written_out() {
        // `1,...,4` is four values, not three tokens.
        assert_eq!(values("1,...,4"), vec!["1", "2", "3", "4"]);
        // The step comes from the first two, so `0,2,...,6` counts by twos.
        assert_eq!(values("0,2,...,6"), vec!["0", "2", "4", "6"]);
    }

    #[test]
    fn a_loop_variable_is_not_matched_by_prefix() {
        // `\x` inside `\xshift` is not the loop variable, and substituting it
        // would turn a real option into `1shift`.
        let out = substitute(r"\draw[xshift=\x cm] (\x,0);", "x", "1");
        assert_eq!(out, r"\draw[xshift=1 cm] (1,0);");
        let out = substitute(r"\draw[\xshift] (\x,0);", "x", "1");
        assert_eq!(out, r"\draw[\xshift] (1,0);");
    }
}
