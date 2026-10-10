//! TeX82's primitive control sequences, as `\meaning` and `\show` name them.
//!
//! `tex.web` enters every primitive into the hash table with a `primitive`
//! call (§226 through §1344), so before a document redefines one its meaning
//! is the primitive itself and §296's `print_meaning` prints it through §298's
//! `print_cmd_chr` -- almost always as the escaped name. texrs dispatches only
//! some of them, but which of them it can EXECUTE is a separate fact from what
//! each one MEANS: `\meaning\hsize` is `\hsize` in every TeX, whether or not
//! the engine in front of it lays out a page with it.
//!
//! The list is §226's `primitive` calls in TeX82 (no e-TeX or pdfTeX
//! extensions), measured against tex 3.141592653 one name at a time.

/// Every TeX82 primitive that is not a §236 integer parameter (those are
/// `crate::intpar::NAMES`). The control symbols `\ `, `\/` and `\-` are the
/// one-character names `" "`, `"/"` and `"-"`.
const NAMES: &[&str] = &[
    // §226: the glue parameters.
    "lineskip",
    "baselineskip",
    "parskip",
    "abovedisplayskip",
    "belowdisplayskip",
    "abovedisplayshortskip",
    "belowdisplayshortskip",
    "leftskip",
    "rightskip",
    "topskip",
    "splittopskip",
    "tabskip",
    "spaceskip",
    "xspaceskip",
    "parfillskip",
    "thinmuskip",
    "medmuskip",
    "thickmuskip",
    // §230: the token-list parameters.
    "output",
    "everypar",
    "everymath",
    "everydisplay",
    "everyhbox",
    "everyvbox",
    "everyjob",
    "everycr",
    "errhelp",
    // §248: the dimension parameters.
    "parindent",
    "mathsurround",
    "lineskiplimit",
    "hsize",
    "vsize",
    "maxdepth",
    "splitmaxdepth",
    "boxmaxdepth",
    "hfuzz",
    "vfuzz",
    "delimitershortfall",
    "nulldelimiterspace",
    "scriptspace",
    "predisplaysize",
    "displaywidth",
    "displayindent",
    "overfullrule",
    "hangindent",
    "hoffset",
    "voffset",
    "emergencystretch",
    // §265: the rest of the commands the hash table starts with.
    " ",
    "/",
    "accent",
    "advance",
    "afterassignment",
    "aftergroup",
    "begingroup",
    "char",
    "csname",
    "delimiter",
    "divide",
    "endcsname",
    "endgroup",
    "expandafter",
    "font",
    "fontdimen",
    "halign",
    "hrule",
    "ignorespaces",
    "insert",
    "mark",
    "mathaccent",
    "mathchar",
    "mathchoice",
    "multiply",
    "noalign",
    "noboundary",
    "noexpand",
    "nonscript",
    "omit",
    "parshape",
    "penalty",
    "prevgraf",
    "radical",
    "read",
    "relax",
    "setbox",
    "the",
    "toks",
    "vadjust",
    "valign",
    "vcenter",
    "vrule",
    // §334, §376, §384: \par, \input and \endinput, the five marks.
    "par",
    "input",
    "endinput",
    "topmark",
    "firstmark",
    "botmark",
    "splitfirstmark",
    "splitbotmark",
    // §411, §416: registers and the internal quantities.
    "count",
    "dimen",
    "skip",
    "muskip",
    "spacefactor",
    "prevdepth",
    "deadcycles",
    "insertpenalties",
    "wd",
    "ht",
    "dp",
    "lastpenalty",
    "lastkern",
    "lastskip",
    "inputlineno",
    "badness",
    // §468, §487, §491: conversions and conditionals.
    "number",
    "romannumeral",
    "string",
    "meaning",
    "fontname",
    "jobname",
    "if",
    "ifcat",
    "ifnum",
    "ifdim",
    "ifodd",
    "ifvmode",
    "ifhmode",
    "ifmmode",
    "ifinner",
    "ifvoid",
    "ifhbox",
    "ifvbox",
    "ifx",
    "ifeof",
    "iftrue",
    "iffalse",
    "ifcase",
    "fi",
    "or",
    "else",
    // §553, §780: the null font and the alignment delimiters.
    "nullfont",
    "span",
    "cr",
    "crcr",
    // §983, §1052 onward: the page builder and the chief executive.
    "pagegoal",
    "pagetotal",
    "pagestretch",
    "pagefilstretch",
    "pagefillstretch",
    "pagefilllstretch",
    "pageshrink",
    "pagedepth",
    "end",
    "dump",
    "hskip",
    "hfil",
    "hfill",
    "hss",
    "hfilneg",
    "vskip",
    "vfil",
    "vfill",
    "vss",
    "vfilneg",
    "mskip",
    "kern",
    "mkern",
    "moveleft",
    "moveright",
    "raise",
    "lower",
    "box",
    "copy",
    "lastbox",
    "vsplit",
    "vtop",
    "vbox",
    "hbox",
    "shipout",
    "leaders",
    "cleaders",
    "xleaders",
    "indent",
    "noindent",
    "unpenalty",
    "unkern",
    "unskip",
    "unhbox",
    "unhcopy",
    "unvbox",
    "unvcopy",
    "-",
    "discretionary",
    "eqno",
    "leqno",
    "mathord",
    "mathop",
    "mathbin",
    "mathrel",
    "mathopen",
    "mathclose",
    "mathpunct",
    "mathinner",
    "underline",
    "overline",
    "displaylimits",
    "limits",
    "nolimits",
    "displaystyle",
    "textstyle",
    "scriptstyle",
    "scriptscriptstyle",
    "above",
    "over",
    "atop",
    "abovewithdelims",
    "overwithdelims",
    "atopwithdelims",
    "left",
    "right",
    "long",
    "outer",
    "global",
    "def",
    "gdef",
    "edef",
    "xdef",
    "let",
    "futurelet",
    "chardef",
    "mathchardef",
    "countdef",
    "dimendef",
    "skipdef",
    "muskipdef",
    "toksdef",
    "catcode",
    "mathcode",
    "lccode",
    "uccode",
    "sfcode",
    "delcode",
    "textfont",
    "scriptfont",
    "scriptscriptfont",
    "hyphenation",
    "patterns",
    "hyphenchar",
    "skewchar",
    "batchmode",
    "nonstopmode",
    "scrollmode",
    "errorstopmode",
    "openin",
    "closein",
    "message",
    "errmessage",
    "lowercase",
    "uppercase",
    "show",
    "showbox",
    "showthe",
    "showlists",
    // §1344: the extensions.
    "openout",
    "write",
    "closeout",
    "special",
    "immediate",
    "setlanguage",
];

/// Whether `name` is a TeX82 primitive outside §236's integer parameters.
pub fn is_primitive(name: &str) -> bool {
    NAMES.contains(&name)
}

/// §413's internal quantities that are not parameters: the primitives whose
/// commands lie from §209's `min_internal` to `max_internal` -- `last_item`,
/// `toks_register`, `assign_font_dimen`, `assign_font_int`, `set_aux`,
/// `set_prev_graf`, `set_page_dimen`, `set_page_int`, `set_box_dimen`,
/// `set_shape`, `def_code`, `def_family`, `set_font`, `def_font` and
/// `register`. A `\chardef`, `\mathchardef` or register name is one too, by
/// its meaning rather than by a name here.
const INTERNAL: &[&str] = &[
    "lastpenalty",
    "lastkern",
    "lastskip",
    "inputlineno",
    "badness",
    "toks",
    "fontdimen",
    "hyphenchar",
    "skewchar",
    "spacefactor",
    "prevdepth",
    "prevgraf",
    "pagegoal",
    "pagetotal",
    "pagestretch",
    "pagefilstretch",
    "pagefillstretch",
    "pagefilllstretch",
    "pageshrink",
    "pagedepth",
    "deadcycles",
    "insertpenalties",
    "wd",
    "ht",
    "dp",
    "parshape",
    "catcode",
    "mathcode",
    "lccode",
    "uccode",
    "sfcode",
    "delcode",
    "textfont",
    "scriptfont",
    "scriptscriptfont",
    "nullfont",
    "font",
    "count",
    "dimen",
    "skip",
    "muskip",
];

/// Whether the primitive `name` is an internal quantity (§413), which §440's
/// `scan_int` reads through `scan_something_internal` rather than as the
/// missing number it would otherwise be: one of `INTERNAL`, or a glue,
/// token-list, dimension (§226, §230, §248) or integer (§236) parameter.
pub fn is_internal_quantity(name: &str) -> bool {
    // The parameter sections open `NAMES` and §265's commands follow them,
    // the first of those being the control space.
    let parameters = NAMES.iter().position(|n| *n == " ").unwrap_or(0);
    INTERNAL.contains(&name)
        || NAMES[..parameters].contains(&name)
        || crate::intpar::index(name).is_some()
}

/// The primitives other than the conditionals that expand (§366): the
/// `expand_after`, `no_expand`, `cs_name`, `convert`, `the`, `top_bot_mark`,
/// `input` and `fi_or_else` commands, and e-TeX's additions to them.
const EXPANDABLE: &[&str] = &[
    "expandafter",
    "unless",
    "noexpand",
    "csname",
    "number",
    "romannumeral",
    "string",
    "meaning",
    "fontname",
    "jobname",
    "csstring",
    "the",
    "detokenize",
    "unexpanded",
    "topmark",
    "firstmark",
    "botmark",
    "splitfirstmark",
    "splitbotmark",
    "input",
    "endinput",
    "scantokens",
    "fi",
    "or",
    "else",
];

/// Whether the primitive `name` expands, the conditionals aside (those are
/// `crate::expand`'s `CONDITIONALS`).
pub fn expands(name: &str) -> bool {
    EXPANDABLE.contains(&name)
}

/// What §296's `print_meaning` prints for the primitive `name` while it still
/// means itself, `esc` being the `\escapechar` character (if any).
///
/// Two kinds print something other than the escaped name. §577's
/// `print_cmd_chr` for `set_font` is `select font` and the font's name, and
/// the only font a primitive names is `\nullfont`. And §296 follows a mark
/// command with `:`, a line end and the mark's text -- empty, because texrs
/// has no page builder to fill one. The line end is NOT written: the message
/// printer has no way to say "end the line" other than the `\newlinechar`
/// character, and a raw line feed would print as `^^J` under INITEX's
/// `\newlinechar=-1`. `BUGS.md` records it.
pub fn meaning(name: &str, esc: &str) -> String {
    match name {
        "nullfont" => "select font nullfont".to_string(),
        "topmark" | "firstmark" | "botmark" | "splitfirstmark" | "splitbotmark" => {
            format!("{esc}{name}:")
        }
        _ => format!("{esc}{name}"),
    }
}

/// Whether `name` is something tex starts out knowing: a TeX82 primitive, a §236
/// parameter, or a command this engine documents. Anything else is undefined
/// until a document defines it.
///
/// A set rather than a scan of the lists: the expander asks for every control
/// sequence it expands.
pub fn is_known_name(name: &str) -> bool {
    use once_cell::sync::Lazy;
    static KNOWN: Lazy<std::collections::HashSet<&'static str>> = Lazy::new(|| {
        let mut set: std::collections::HashSet<&'static str> = NAMES.iter().copied().collect();
        set.extend(crate::intpar::NAMES.iter().copied());
        set.extend(crate::corpus::names().filter_map(|n| n.strip_prefix('\\')));
        set
    });
    KNOWN.contains(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_name_is_listed_twice_or_also_an_integer_parameter() {
        for (i, n) in NAMES.iter().enumerate() {
            assert!(!NAMES[..i].contains(n), "{n} listed twice");
            assert!(crate::intpar::index(n).is_none(), "{n} is a §236 parameter");
        }
    }
}
