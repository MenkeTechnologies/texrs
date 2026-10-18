//! What a `\font` request means to luaotfload.
//!
//! A LuaLaTeX `\font\x="file:Arimo-Regular.ttf:+liga;-kern;mode=harf"` is not
//! a file name in TeX's sense: the string goes to luaotfload's `define_font`
//! callback, which reads it with the `font_request` grammar of
//! `luaotfload-parsers.lua` (luaotfload 3.29, lines 420-642) and then turns
//! the result into a specification in `analyze` (`luaotfload-features.lua`
//! :354-460). This is a port of both. The grammar is an LPEG expression, so
//! it is ported as the parsing expression it is -- ordered choice, no
//! backtracking into a choice once it has matched, and no requirement that the
//! whole string be consumed (`font_request` has no `* -1`, so whatever follows
//! a complete request is ignored, exactly as luaotfload ignores it).
//!
//! `combo:` requests (`luaotfload-features.lua:68`'s font combinations) are
//! recognised and refused by name rather than parsed: they build a virtual
//! font out of other fonts, and there is no virtual-font constructor to hand
//! them to.

use std::collections::BTreeMap;

use crate::shape::FeatureValue;

/// The lookup a request asks for (luaotfload-parsers.lua:449-470).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// `name:` -- the font names database, then file names.
    Name,
    /// `file:` -- a file name, through the database's file index and kpse.
    File,
    /// `kpse:` -- kpathsea alone.
    Kpse,
    /// `my:` -- the `luaotfload.resolve_font` callback.
    My,
    /// `id:` -- a font already defined, by its id.
    Id,
    /// `[...]` -- a path, verbatim.
    Path,
    /// No prefix: TFM/OFM first, then a path, then a name (XeTeX's syntax).
    Anon,
}

/// Which face of a collection: `(2)` is the third face, stored as luaotfload
/// stores it, plus one (parsers.lua:577-580); anything else between the
/// parentheses is kept as a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubFont {
    Index(u32),
    Name(String),
}

/// A slash modifier (parsers.lua:509-520): a style, an optical size, or
/// something luaotfload reads and drops.
#[derive(Clone, Debug, PartialEq)]
pub enum Modifier {
    /// `b`, `i`, `bi`, `ib` or `r`, lower-cased.
    Style(String),
    /// `S=12`: the optical size, as written.
    OptSize(String),
    /// `AAT`, `ICU`, `GR`: matched and ignored.
    Ignored,
    /// Anything else: `false` in the Lua, reported and ignored.
    Garbage,
}

/// The table `font_request` captures.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Request {
    pub lookup: Option<Lookup>,
    pub name: String,
    pub id: Option<u32>,
    pub sub: Option<SubFont>,
    pub modifiers: Vec<Modifier>,
    /// The features, in luaotfload's `rawset` fold: a later setting of the
    /// same key replaces the earlier one.
    pub features: BTreeMap<String, FeatureValue>,
}

/// A request luaotfload can name the reason for refusing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// `combo:`, which this port does not build.
    Combo,
}

struct P<'a> {
    s: &'a [u8],
    at: usize,
}

impl<'a> P<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn lit(&mut self, t: &str) -> bool {
        if self.s[self.at..].starts_with(t.as_bytes()) {
            self.at += t.len();
            true
        } else {
            false
        }
    }

    /// `ws = S" \t\v"^0` (parsers.lua:73-76).
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | 0x0B)) {
            self.at += 1;
        }
    }

    fn text(&self, from: usize) -> String {
        String::from_utf8_lossy(&self.s[from..self.at]).into_owned()
    }

    /// `fontname = C((1-S":(/")^1)`.
    fn fontname(&mut self) -> Option<String> {
        let from = self.at;
        while matches!(self.peek(), Some(c) if !b":(/".contains(&c)) {
            self.at += 1;
        }
        (self.at > from).then(|| self.text(from))
    }

    /// `balanced_braces = P{((1 - S'{}') + '{' * V(1) * '}')^0}`: always
    /// matches, possibly nothing.
    fn balanced_braces(&mut self) {
        loop {
            match self.peek() {
                Some(b'{') => {
                    let save = self.at;
                    self.at += 1;
                    self.balanced_braces();
                    if self.peek() == Some(b'}') {
                        self.at += 1;
                    } else {
                        self.at = save;
                        return;
                    }
                }
                Some(b'}') | None => return,
                Some(_) => self.at += 1,
            }
        }
    }

    /// `'{' * balanced_braces * '}'`, consuming; false (and no movement)
    /// when the braces do not balance.
    fn braced(&mut self) -> bool {
        let save = self.at;
        if self.peek() != Some(b'{') {
            return false;
        }
        self.at += 1;
        self.balanced_braces();
        if self.peek() == Some(b'}') {
            self.at += 1;
            true
        } else {
            self.at = save;
            false
        }
    }

    /// `field = '{' * C(balanced_braces) * '}' + C(field_char^1)`.
    fn field(&mut self) -> Option<String> {
        let save = self.at;
        if self.peek() == Some(b'{') {
            self.at += 1;
            let from = self.at;
            self.balanced_braces();
            if self.peek() == Some(b'}') {
                let inner = self.text(from);
                self.at += 1;
                return Some(inner);
            }
            self.at = save;
        }
        // `field_char = '{' * balanced_braces * '}' + (1 - S'={}' - featuresep)`
        let from = self.at;
        loop {
            if self.braced() {
                continue;
            }
            match self.peek() {
                Some(c) if !b"={},;".contains(&c) => self.at += 1,
                _ => break,
            }
        }
        (self.at > from).then(|| self.text(from))
    }

    /// `normal_option = field * ws * equals * ws * field`.
    fn normal_option(&mut self) -> Option<(String, FeatureValue)> {
        let save = self.at;
        let k = self.field();
        if let Some(k) = k {
            self.ws();
            if self.lit("=") {
                self.ws();
                if let Some(v) = self.field() {
                    return Some((k, FeatureValue::Text(v)));
                }
            }
        }
        self.at = save;
        None
    }

    /// `feature_expr = ws * Cg(assignment + switch) * ws`.
    fn option(&mut self) -> Option<(String, FeatureValue)> {
        let save = self.at;
        self.ws();
        let got = self.assignment().or_else(|| self.switch());
        match got {
            Some(kv) => {
                self.ws();
                Some(kv)
            }
            None => {
                self.at = save;
                None
            }
        }
    }

    /// `assignment = xetex_option + normal_option`, where
    /// `xetex_option = P"+" * ws * (field * ws * equals * ws *
    /// (digit^1/handle_xetex_option) + normal_option)`.
    fn assignment(&mut self) -> Option<(String, FeatureValue)> {
        let save = self.at;
        if self.lit("+") {
            self.ws();
            let inner = self.at;
            if let Some(k) = self.field() {
                self.ws();
                if self.lit("=") {
                    self.ws();
                    let from = self.at;
                    while matches!(self.peek(), Some(b'0'..=b'9')) {
                        self.at += 1;
                    }
                    if self.at > from {
                        // XeTeX counts from zero (parsers.lua:494-496).
                        let n: u64 = self.text(from).parse().unwrap_or(0);
                        return Some((k, FeatureValue::Text((n + 1).to_string())));
                    }
                }
            }
            self.at = inner;
            if let Some(kv) = self.normal_option() {
                return Some(kv);
            }
            self.at = save;
        }
        self.normal_option()
    }

    /// `switch = P"-" * ws * field * Cc(false) + P"+"^-1 * ws * field * Cc(true)`.
    fn switch(&mut self) -> Option<(String, FeatureValue)> {
        let save = self.at;
        if self.lit("-") {
            self.ws();
            if let Some(k) = self.field() {
                return Some((k, FeatureValue::Bool(false)));
            }
            self.at = save;
        }
        self.lit("+");
        self.ws();
        match self.field() {
            Some(k) => Some((k, FeatureValue::Bool(true))),
            None => {
                self.at = save;
                None
            }
        }
    }

    fn featuresep(&mut self) -> bool {
        self.lit(",") || self.lit(";")
    }

    /// `feature_list = Cf(Ct"" * option * (featuresep * option^-1)^0, rawset)
    /// * featuresep^-1`.
    fn feature_list(&mut self) -> Option<BTreeMap<String, FeatureValue>> {
        let mut out = BTreeMap::new();
        let (k, v) = self.option()?;
        out.insert(k, v);
        loop {
            let save = self.at;
            if !self.featuresep() {
                break;
            }
            match self.option() {
                Some((k, v)) => {
                    out.insert(k, v);
                }
                None => {
                    // `option^-1` matched nothing; the separator stands.
                    let _ = save;
                }
            }
        }
        self.featuresep();
        Some(out)
    }

    /// `modifier = slash * (other_modifier + Cs(style_modifier) +
    /// Ct(size_modifier) + garbage_modifier)`.
    fn modifier(&mut self) -> Option<Modifier> {
        if !self.lit("/") {
            return None;
        }
        for other in ["AAT", "aat", "ICU", "icu", "GR", "gr"] {
            if self.lit(other) {
                return Some(Modifier::Ignored);
            }
        }
        // `S'bB' * S'iI'^-1 + S'iI' * S'bB'^-1 + S'rR'`, lower-cased.
        let from = self.at;
        match self.peek() {
            Some(b'b' | b'B') => {
                self.at += 1;
                if matches!(self.peek(), Some(b'i' | b'I')) {
                    self.at += 1;
                }
            }
            Some(b'i' | b'I') => {
                self.at += 1;
                if matches!(self.peek(), Some(b'b' | b'B')) {
                    self.at += 1;
                }
            }
            Some(b'r' | b'R') => self.at += 1,
            _ => {}
        }
        if self.at > from {
            return Some(Modifier::Style(self.text(from).to_lowercase()));
        }
        // `S"Ss" * P"=" * Cc"optsize" * C(decimal)`
        let save = self.at;
        if matches!(self.peek(), Some(b'S' | b's')) {
            self.at += 1;
            if self.lit("=") {
                let from = self.at;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.at += 1;
                }
                if self.at > from {
                    if self.lit(".") {
                        while matches!(self.peek(), Some(b'0'..=b'9')) {
                            self.at += 1;
                        }
                    }
                    return Some(Modifier::OptSize(self.text(from)));
                }
            }
            self.at = save;
        }
        // `(1 - colon - slash)^0 * Cc(false)`
        while matches!(self.peek(), Some(c) if c != b':' && c != b'/') {
            self.at += 1;
        }
        Some(Modifier::Garbage)
    }

    /// `path_balanced = { (path_content + V(2))^1, lbrk * V(1)^-1 * rbrk }`,
    /// with `path_content = backslash / "" * utf8char + (1 - brackets)`:
    /// the substitution capture drops each escaping backslash.
    fn path_balanced(&mut self, out: &mut Vec<u8>) -> bool {
        let start = self.at;
        loop {
            match self.peek() {
                Some(b'\\') if self.at + 1 < self.s.len() => {
                    self.at += 1;
                    let len = utf8_len(self.s[self.at]);
                    let end = (self.at + len).min(self.s.len());
                    out.extend_from_slice(&self.s[self.at..end]);
                    self.at = end;
                }
                Some(b'[') => {
                    let save = (self.at, out.len());
                    self.at += 1;
                    out.push(b'[');
                    self.path_balanced(out);
                    if self.lit("]") {
                        out.push(b']');
                    } else {
                        self.at = save.0;
                        out.truncate(save.1);
                        break;
                    }
                }
                Some(b']') | None => break,
                Some(c) => {
                    out.push(c);
                    self.at += 1;
                }
            }
        }
        self.at > start
    }
}

fn utf8_len(b: u8) -> usize {
    match b {
        0xF0..=0xF7 => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

/// `font_request` over a request string. `None` is LPEG's no-match, which
/// `analyze` turns into a path lookup of the whole string.
pub fn parse(spec: &str) -> Result<Option<Request>, RequestError> {
    let mut p = P {
        s: spec.as_bytes(),
        at: 0,
    };
    if p.s.starts_with(b"combo") && {
        let mut q = P { s: p.s, at: 5 };
        q.lit(":")
    } {
        return Err(RequestError::Combo);
    }
    let mut req = Request::default();
    // `specification = (path_lookup + prefixed + unprefixed) * maybe_subfont
    // * modifier_list`
    let matched = 'spec: {
        if p.peek() == Some(b'[') {
            let save = p.at;
            p.at += 1;
            let mut name = Vec::new();
            if p.path_balanced(&mut name) && p.lit("]") {
                req.lookup = Some(Lookup::Path);
                req.name = String::from_utf8_lossy(&name).into_owned();
                break 'spec true;
            }
            p.at = save;
        }
        for (prefix, lookup) in [
            ("name", Lookup::Name),
            ("file", Lookup::File),
            ("kpse", Lookup::Kpse),
            ("my", Lookup::My),
        ] {
            let save = p.at;
            if p.lit(prefix) && p.lit(":") {
                p.ws();
                if let Some(name) = p.fontname() {
                    req.lookup = Some(lookup);
                    req.name = name;
                    break 'spec true;
                }
            }
            p.at = save;
        }
        {
            let save = p.at;
            if p.lit("id") && p.lit(":") {
                p.ws();
                let from = p.at;
                while matches!(p.peek(), Some(b'0'..=b'9')) {
                    p.at += 1;
                }
                if p.at > from {
                    req.lookup = Some(Lookup::Id);
                    req.id = p.text(from).parse().ok();
                    break 'spec true;
                }
            }
            p.at = save;
        }
        match p.fontname() {
            Some(name) => {
                req.lookup = Some(Lookup::Anon);
                req.name = name;
                true
            }
            None => false,
        }
    };
    if !matched {
        return Ok(None);
    }
    // `maybe_subfont = subfont + #(1 - P"(" + -1)`
    if p.peek() == Some(b'(') {
        p.at += 1;
        let from = p.at;
        while matches!(p.peek(), Some(b'0'..=b'9')) {
            p.at += 1;
        }
        let digits = p.at > from;
        if digits && p.peek() == Some(b')') {
            let n: u32 = p.text(from).parse().unwrap_or(0);
            req.sub = Some(SubFont::Index(n + 1));
            p.at += 1;
        } else {
            p.at = from;
            while matches!(p.peek(), Some(c) if c != b'(' && c != b')') {
                p.at += 1;
            }
            if p.at > from && p.peek() == Some(b')') {
                req.sub = Some(SubFont::Name(p.text(from)));
                p.at += 1;
            } else {
                // A `(` that opens no subfont: the specification fails.
                return Ok(None);
            }
        }
    }
    while let Some(m) = p.modifier() {
        req.modifiers.push(m);
    }
    // `(colon^-1 * features)^-1`; nothing after this is read.
    p.lit(":");
    if let Some(f) = p.feature_list() {
        req.features = f;
    }
    Ok(Some(req))
}

/// `analyze`'s specification (luaotfload-features.lua:354-460), the fields a
/// font loader reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Specification {
    /// The request as written: luaotfload's `specification.specification`,
    /// which is also the font's name.
    pub specification: String,
    pub lookup: Lookup,
    pub name: String,
    pub id: Option<u32>,
    pub sub: Option<SubFont>,
    /// `b`, `i`, `bi` or `r` from the slash modifiers (`handle_slashed`).
    pub style: Option<String>,
    pub optsize: Option<f64>,
    /// `features.raw`: the request's own features, untouched by defaults.
    /// This is what `mode=harf` shapes with.
    pub raw: BTreeMap<String, FeatureValue>,
}

impl Specification {
    /// `features.raw.mode`, which decides the loader: `harf` (and `plug`,
    /// its spelling inside the fontloader) for HarfBuzz, otherwise node mode.
    pub fn mode(&self) -> Option<&str> {
        match self.raw.get("mode") {
            Some(FeatureValue::Text(m)) => Some(m.as_str()),
            _ => None,
        }
    }

    fn raw_text(&self, key: &str) -> Option<&str> {
        match self.raw.get(key) {
            Some(FeatureValue::Text(t)) => Some(t.as_str()),
            _ => None,
        }
    }

    /// `features.raw.script`.
    pub fn script(&self) -> Option<&str> {
        self.raw_text("script")
    }

    /// `features.raw.language`.
    pub fn language(&self) -> Option<&str> {
        self.raw_text("language")
    }
}

/// `analyze(spec_string, size)` without the font database: the parse, the
/// fallback to a path lookup when the grammar does not match
/// (features.lua:357-394), the slash modifiers, and `lookup`/`sub` imported
/// from the feature list (:446-457).
pub fn analyze(spec: &str) -> Result<Specification, RequestError> {
    let Some(req) = parse(spec)? else {
        // features.lua:375-394: a bare path, perhaps ending in `(N)`.
        let (name, sub) = match spec.rfind('(') {
            Some(open) if spec.ends_with(')') && open + 1 < spec.len() - 1 => {
                let inner = &spec[open + 1..spec.len() - 1];
                if inner.contains('(') || inner.contains(')') {
                    (spec.to_string(), None)
                } else {
                    let sub = match inner.parse::<u32>() {
                        Ok(n) => SubFont::Index(n),
                        Err(_) => SubFont::Name(inner.to_string()),
                    };
                    (spec[..open].to_string(), Some(sub))
                }
            }
            _ => (spec.to_string(), None),
        };
        return Ok(Specification {
            specification: spec.to_string(),
            lookup: Lookup::Path,
            name,
            id: None,
            sub,
            style: None,
            optsize: None,
            raw: BTreeMap::new(),
        });
    };
    // `handle_slashed` (features.lua:326-344): the last style wins.
    let (mut style, mut optsize) = (None, None);
    for m in &req.modifiers {
        match m {
            Modifier::OptSize(s) => optsize = s.parse().ok(),
            Modifier::Style(s) => style = Some(s.clone()),
            Modifier::Ignored | Modifier::Garbage => {}
        }
    }
    let mut lookup = req.lookup.unwrap_or(Lookup::Anon);
    let mut sub = req.sub.clone();
    // import_values: a `lookup=` or `sub=` among the features overrides.
    if let Some(FeatureValue::Text(l)) = req.features.get("lookup") {
        lookup = match l.as_str() {
            "name" => Lookup::Name,
            "file" => Lookup::File,
            "kpse" => Lookup::Kpse,
            "my" => Lookup::My,
            "id" => Lookup::Id,
            "path" => Lookup::Path,
            _ => Lookup::Anon,
        };
    }
    if let Some(FeatureValue::Text(s)) = req.features.get("sub") {
        sub = Some(match s.parse::<u32>() {
            Ok(n) => SubFont::Index(n),
            Err(_) => SubFont::Name(s.clone()),
        });
    }
    Ok(Specification {
        specification: spec.to_string(),
        lookup,
        name: req.name,
        id: req.id,
        sub,
        style,
        optsize,
        raw: req.features,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feats(spec: &str) -> Vec<(String, FeatureValue)> {
        parse(spec).unwrap().unwrap().features.into_iter().collect()
    }

    /// The canonical form, as fontspec writes it.
    #[test]
    fn file_request_with_switches_and_options() {
        let r = parse("file:Arimo-Regular.ttf:+liga;-kern;mode=harf;script=latn")
            .unwrap()
            .unwrap();
        assert_eq!(r.lookup, Some(Lookup::File));
        assert_eq!(r.name, "Arimo-Regular.ttf");
        assert_eq!(r.features["liga"], FeatureValue::Bool(true));
        assert_eq!(r.features["kern"], FeatureValue::Bool(false));
        assert_eq!(r.features["mode"], FeatureValue::Text("harf".into()));
        assert_eq!(r.features["script"], FeatureValue::Text("latn".into()));
    }

    /// `fontname` stops at `:`, `(` and `/`; a subfont is counted from one
    /// once parsed; the modifiers follow it.
    #[test]
    fn subfont_and_modifiers() {
        let r = parse("name:Cambria(1)/BI:+smcp").unwrap().unwrap();
        assert_eq!(r.lookup, Some(Lookup::Name));
        assert_eq!(r.name, "Cambria");
        assert_eq!(r.sub, Some(SubFont::Index(2)));
        assert_eq!(r.modifiers, vec![Modifier::Style("bi".into())]);
        assert_eq!(r.features["smcp"], FeatureValue::Bool(true));
        let s = analyze("Latin Modern Roman/I/S=12").unwrap();
        assert_eq!(s.lookup, Lookup::Anon);
        assert_eq!(s.style.as_deref(), Some("i"));
        assert_eq!(s.optsize, Some(12.0));
    }

    /// Brackets take a path with any character but an unbalanced bracket,
    /// and a backslash escapes the next character.
    #[test]
    fn bracketed_path() {
        let r = parse(r"[./fonts/My\]Font.otf]:+kern").unwrap().unwrap();
        assert_eq!(r.lookup, Some(Lookup::Path));
        assert_eq!(r.name, "./fonts/My]Font.otf");
        assert_eq!(r.features["kern"], FeatureValue::Bool(true));
    }

    /// XeTeX's `+key=N` counts alternates from zero; luaotfload adds one.
    /// Commas separate as semicolons do, a later key wins, and a braced
    /// field may hold separators.
    #[test]
    fn feature_list_forms() {
        let f = feats("file:x.otf:+salt=0,kern=2;liga;color={1,2};liga=false;");
        assert_eq!(
            f,
            vec![
                ("color".into(), FeatureValue::Text("1,2".into())),
                ("kern".into(), FeatureValue::Text("2".into())),
                ("liga".into(), FeatureValue::Text("false".into())),
                ("salt".into(), FeatureValue::Text("1".into())),
            ]
        );
    }

    /// A string the grammar does not match at all is a path
    /// (features.lua:375-394), its trailing `(N)` a subfont.
    #[test]
    fn unmatched_request_is_a_path() {
        let s = analyze(":weird(3)").unwrap();
        assert_eq!(s.lookup, Lookup::Path);
        assert_eq!(s.name, ":weird");
        assert_eq!(s.sub, Some(SubFont::Index(3)));
    }

    #[test]
    fn combo_is_refused_by_name() {
        assert_eq!(parse("combo: 1->42; 2->99,0x41"), Err(RequestError::Combo));
    }
}
