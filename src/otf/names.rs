//! Font requests resolved to files as luaotfload resolves them
//! (luaotfload-resolvers.lua and the lookup half of luaotfload-database.lua):
//! `file:` and `path:` lookups, `name:` lookups through luaotfload's font
//! names database, and the `anon` sequence an unprefixed request takes
//! (`tex`, `path`, `name`).
//!
//! The database is the one luaotfload keeps (`luaotfload-names.lua`, gzipped,
//! in the `luatex-cache/generic/names` directory of `TEXMFCACHE`), read as
//! luaotfload reads it -- it is a Lua chunk returning a table -- together
//! with its lookup cache, which `name:` requests consult first as
//! luaotfload's default `cached` resolver does. Building the database, and
//! rebuilding it when a lookup misses (`update_live`), are not ported: a
//! machine whose luaotfload has never built one resolves no `name:`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One face the database knows (`mappings[i]`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mapping {
    pub basename: String,
    pub fullpath: String,
    pub location: String,
    pub format: String,
    pub subfont: Option<u32>,
    pub psname: Option<String>,
}

/// A family style's faces: (design size, top, bottom, index) and the
/// default one (`families[loc][fmt][family][style]`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sizes {
    pub sizes: Vec<(f64, f64, f64, usize)>,
    pub default: Option<usize>,
}

type ByLocFmt<T> = BTreeMap<String, BTreeMap<String, T>>;

#[derive(Clone, Debug, Default)]
pub struct Db {
    /// 1-based in the Lua; `mappings[0]` here is index 1.
    pub mappings: Vec<Mapping>,
    /// `files.base[location][basename]`.
    pub base: BTreeMap<String, BTreeMap<String, usize>>,
    /// `files.bare[location][format][barename]`.
    pub bare: ByLocFmt<BTreeMap<String, usize>>,
    /// `files.full[index]`.
    pub full: BTreeMap<usize, String>,
    /// `families[location][format][family][style]`.
    pub families: ByLocFmt<BTreeMap<String, BTreeMap<String, Sizes>>>,
    /// `fontnames[location][format][name]`.
    pub fontnames: ByLocFmt<BTreeMap<String, usize>>,
    /// The lookup cache: request hash to file and subfont.
    pub lookups: BTreeMap<String, (String, Option<u32>)>,
}

/// luaotfload-database.lua:216-222.
const FORMAT_PRECEDENCE: [&str; 5] = ["otf", "ttc", "ttf", "afm", "pfb"];
const LOCATION_PRECEDENCE: [&str; 3] = ["local", "system", "texmf"];

/// The directories `getreadablecachepaths` reads: each of TEXMFCACHE's.
fn cache_dirs() -> Vec<PathBuf> {
    let value = crate::kpse::query("luatex", "-var-value", "TEXMFCACHE").unwrap_or_default();
    value
        .split([':', ';'])
        .filter(|s| !s.is_empty())
        .map(|s| Path::new(s).join("luatex-cache/generic/names"))
        .collect()
}

/// `load_maybe_gzip` for the text chunk: `.lua.gz`, else `.lua`.
fn read_chunk(path: &Path) -> Option<String> {
    use std::io::Read;
    let gz = path.with_extension("lua.gz");
    if let Ok(bytes) = std::fs::read(&gz) {
        let mut s = String::new();
        flate2::read::GzDecoder::new(&bytes[..])
            .read_to_string(&mut s)
            .ok()?;
        return Some(s);
    }
    std::fs::read_to_string(path).ok()
}

fn lua_str(v: &mlua::Value) -> Option<String> {
    match v {
        mlua::Value::String(s) => Some(s.to_string_lossy().to_string()),
        _ => None,
    }
}

fn lua_num(v: &mlua::Value) -> Option<f64> {
    match v {
        mlua::Value::Integer(i) => Some(*i as f64),
        mlua::Value::Number(n) => Some(*n),
        _ => None,
    }
}

fn pairs(t: &mlua::Table) -> Vec<(mlua::Value, mlua::Value)> {
    t.pairs::<mlua::Value, mlua::Value>().flatten().collect()
}

fn table_of(v: &mlua::Value) -> Option<&mlua::Table> {
    match v {
        mlua::Value::Table(t) => Some(t),
        _ => None,
    }
}

/// name -> index tables, nested by location and format as `nest` deep.
fn name_index(t: &mlua::Table) -> BTreeMap<String, usize> {
    pairs(t)
        .iter()
        .filter_map(|(k, v)| Some((lua_str(k)?, lua_num(v)? as usize)))
        .collect()
}

impl Db {
    fn from_lua(names: &mlua::Table, lookups: Option<&mlua::Table>) -> Db {
        let mut db = Db::default();
        if let Ok(mlua::Value::Table(m)) = names.get::<mlua::Value>("mappings") {
            for e in m.sequence_values::<mlua::Table>().flatten() {
                let s = |k: &str| e.get::<mlua::Value>(k).ok().and_then(|v| lua_str(&v));
                db.mappings.push(Mapping {
                    basename: s("basename").unwrap_or_default(),
                    fullpath: s("fullpath").unwrap_or_default(),
                    location: s("location").unwrap_or_default(),
                    format: s("format").unwrap_or_default(),
                    subfont: e
                        .get::<mlua::Value>("subfont")
                        .ok()
                        .and_then(|v| lua_num(&v))
                        .map(|n| n as u32),
                    psname: s("psname"),
                });
            }
        }
        if let Ok(mlua::Value::Table(files)) = names.get::<mlua::Value>("files") {
            if let Ok(mlua::Value::Table(base)) = files.get::<mlua::Value>("base") {
                for (loc, t) in pairs(&base) {
                    if let (Some(loc), Some(t)) = (lua_str(&loc), table_of(&t)) {
                        db.base.insert(loc, name_index(t));
                    }
                }
            }
            if let Ok(mlua::Value::Table(bare)) = files.get::<mlua::Value>("bare") {
                for (loc, t) in pairs(&bare) {
                    if let (Some(loc), Some(t)) = (lua_str(&loc), table_of(&t)) {
                        let mut by = BTreeMap::new();
                        for (fmt, m) in pairs(t) {
                            if let (Some(fmt), Some(m)) = (lua_str(&fmt), table_of(&m)) {
                                by.insert(fmt, name_index(m));
                            }
                        }
                        db.bare.insert(loc, by);
                    }
                }
            }
            if let Ok(mlua::Value::Table(full)) = files.get::<mlua::Value>("full") {
                for (k, v) in pairs(&full) {
                    if let (Some(k), Some(v)) = (lua_num(&k), lua_str(&v)) {
                        db.full.insert(k as usize, v);
                    }
                }
            }
        }
        if let Ok(mlua::Value::Table(fam)) = names.get::<mlua::Value>("families") {
            for (loc, t) in pairs(&fam) {
                let (Some(loc), Some(t)) = (lua_str(&loc), table_of(&t)) else {
                    continue;
                };
                let mut by = BTreeMap::new();
                for (fmt, fams) in pairs(t) {
                    let (Some(fmt), Some(fams)) = (lua_str(&fmt), table_of(&fams)) else {
                        continue;
                    };
                    let mut families = BTreeMap::new();
                    for (name, styles) in pairs(fams) {
                        let (Some(name), Some(styles)) = (lua_str(&name), table_of(&styles)) else {
                            continue;
                        };
                        let mut st = BTreeMap::new();
                        for (style, sizes) in pairs(styles) {
                            let (Some(style), Some(sizes)) = (lua_str(&style), table_of(&sizes))
                            else {
                                continue;
                            };
                            let default = sizes
                                .get::<mlua::Value>("default")
                                .ok()
                                .and_then(|v| lua_num(&v))
                                .map(|n| n as usize);
                            let list = sizes
                                .sequence_values::<mlua::Table>()
                                .flatten()
                                .filter_map(|e| {
                                    let n = |i: i64| {
                                        e.get::<mlua::Value>(i).ok().and_then(|v| lua_num(&v))
                                    };
                                    Some((n(1)?, n(2)?, n(3)?, n(4)? as usize))
                                })
                                .collect();
                            st.insert(
                                style,
                                Sizes {
                                    sizes: list,
                                    default,
                                },
                            );
                        }
                        families.insert(name, st);
                    }
                    by.insert(fmt, families);
                }
                db.families.insert(loc, by);
            }
        }
        if let Ok(mlua::Value::Table(fnm)) = names.get::<mlua::Value>("fontnames") {
            for (loc, t) in pairs(&fnm) {
                if let (Some(loc), Some(t)) = (lua_str(&loc), table_of(&t)) {
                    let mut by = BTreeMap::new();
                    for (fmt, m) in pairs(t) {
                        if let (Some(fmt), Some(m)) = (lua_str(&fmt), table_of(&m)) {
                            by.insert(fmt, name_index(m));
                        }
                    }
                    db.fontnames.insert(loc, by);
                }
            }
        }
        if let Some(l) = lookups {
            for (k, v) in pairs(l) {
                let (Some(k), Some(v)) = (lua_str(&k), table_of(&v)) else {
                    continue;
                };
                let file = v.get::<mlua::Value>(1).ok().and_then(|x| lua_str(&x));
                let sub = v
                    .get::<mlua::Value>(2)
                    .ok()
                    .and_then(|x| lua_num(&x))
                    .map(|n| n as u32);
                if let Some(file) = file {
                    db.lookups.insert(k, (file, sub));
                }
            }
        }
        db
    }

    /// luaotfload's database, read once per process; `None` where it has
    /// none.
    pub fn get() -> Option<&'static Db> {
        static DB: OnceLock<Option<Db>> = OnceLock::new();
        DB.get_or_init(|| {
            for dir in cache_dirs() {
                let Some(text) = read_chunk(&dir.join("luaotfload-names.lua")) else {
                    continue;
                };
                let lua = mlua::Lua::new();
                let names: mlua::Table = lua.load(&text).eval().ok()?;
                let lookups: Option<mlua::Table> =
                    read_chunk(&dir.join("luaotfload-lookup-cache.lua"))
                        .and_then(|t| lua.load(&t).eval().ok());
                return Some(Db::from_lua(&names, lookups.as_ref()));
            }
            None
        })
        .as_ref()
    }

    /// `lookup_filename` (:690-729).
    fn lookup_filename(&self, filename: &str) -> Option<&str> {
        let lower = filename.to_lowercase();
        for loc in LOCATION_PRECEDENCE {
            let mut idx = self
                .base
                .get(loc)
                .and_then(|b| b.get(filename).or_else(|| b.get(&lower)));
            if idx.is_none() {
                if let Some(bare) = self.bare.get(loc) {
                    for fmt in FORMAT_PRECEDENCE {
                        if let Some(m) = bare.get(fmt) {
                            idx = m.get(filename).or_else(|| m.get(&lower));
                            if idx.is_some() {
                                break;
                            }
                        }
                    }
                }
            }
            if let Some(i) = idx {
                return self.full.get(i).map(String::as_str);
            }
        }
        None
    }

    /// `lookup_fullpath` (:1274-1303) without an extension.
    fn lookup_fullpath(&self, fontname: &str) -> Option<&str> {
        let lower = fontname.to_lowercase();
        for loc in LOCATION_PRECEDENCE {
            if let Some(i) = self
                .base
                .get(loc)
                .and_then(|b| b.get(fontname).or_else(|| b.get(&lower)))
            {
                return self.full.get(i).map(String::as_str);
            }
        }
        None
    }

    /// `get_font_file` (:801-819): a texmf face through kpathsea, another
    /// by its full path.
    fn get_font_file(&self, index: usize) -> Option<(PathBuf, Option<u32>)> {
        let e = self.mappings.get(index.checked_sub(1)?)?;
        if e.location == "texmf" {
            return crate::kpse::locate(&e.basename).map(|p| (p, e.subfont));
        }
        let full = self.full.get(&index)?;
        Path::new(full)
            .is_file()
            .then(|| (PathBuf::from(full), e.subfont))
    }

    /// `lookup_familyname` (:1063-1101): the file, subfont and whether the
    /// style fell back to the family's regular.
    fn lookup_familyname(
        &self,
        name: &str,
        style: &str,
        askedsize: f64,
    ) -> Option<(PathBuf, Option<u32>, bool)> {
        let mut candidates: Option<&Sizes> = None;
        let mut fallback = true;
        'search: for loc in LOCATION_PRECEDENCE {
            let Some(locgroup) = self.families.get(loc) else {
                continue;
            };
            for fmt in FORMAT_PRECEDENCE {
                let Some(family) = locgroup.get(fmt).and_then(|f| f.get(name)) else {
                    continue;
                };
                if let Some(s) = family.get(style) {
                    candidates = Some(s);
                    fallback = false;
                    break 'search;
                } else if candidates.is_none() {
                    candidates = family.get("r");
                }
            }
        }
        let index = choose_size(candidates?, askedsize)?;
        let (path, sub) = self.get_font_file(index)?;
        Some((path, sub, fallback))
    }

    /// `lookup_fontname` (:1103-1123).
    fn lookup_fontname(&self, name: &str) -> Option<(PathBuf, Option<u32>)> {
        for loc in LOCATION_PRECEDENCE {
            let Some(locgroup) = self.fontnames.get(loc) else {
                continue;
            };
            for fmt in FORMAT_PRECEDENCE {
                if let Some(&i) = locgroup.get(fmt).and_then(|m| m.get(name)) {
                    if let Some(found) = self.get_font_file(i) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    /// `lookup_font_name` (:1231-1272), without the rebuild on a miss.
    pub fn lookup_font_name(
        &self,
        name: &str,
        style: Option<&str>,
        optsize: Option<f64>,
        size: Option<i64>,
    ) -> Option<(PathBuf, Option<u32>)> {
        let name = sanitize_fontname(name);
        let style = style.map_or("r".to_string(), sanitize_fontname);
        let askedsize = match optsize {
            Some(o) => o * 65536.0,
            None => size.filter(|s| *s >= 0).unwrap_or(0) as f64,
        };
        // `designsize_dimen` is `bp` by default: no scaling
        let mut resolved = self.lookup_familyname(&name, &style, askedsize);
        if resolved.as_ref().is_none_or(|r| r.2) {
            if let Some((p, s)) = self.lookup_fontname(&name) {
                resolved = Some((p, s, false));
            }
        }
        resolved.map(|(p, s, _)| (p, s))
    }

    /// `verify_font_file` (:827-836).
    fn verify_font_file(&self, basename: &str) -> bool {
        if self
            .lookup_fullpath(basename)
            .is_some_and(|p| Path::new(p).is_file())
        {
            return true;
        }
        // `resolversfindfile` finds a full path that exists
        Path::new(basename).is_file() || crate::kpse::locate(basename).is_some()
    }
}

/// `choose_size` (:1005-1053).
fn choose_size(sizes: &Sizes, askedsize: f64) -> Option<usize> {
    let mut inrange: Vec<(f64, usize)> = Vec::new();
    let mut norange: Vec<(f64, usize)> = Vec::new();
    if askedsize != 0.0 {
        for &(dsnsize, high, low, index) in &sizes.sizes {
            if dsnsize == askedsize {
                return Some(index);
            } else if askedsize <= low {
                norange.push((low - askedsize, index));
            } else if askedsize > high {
                norange.push((askedsize - high, index));
            } else {
                let d = if dsnsize > low && dsnsize < high {
                    dsnsize - askedsize
                } else {
                    (low + high) / 2.0 - askedsize
                };
                inrange.push((d.abs(), index));
            }
        }
    }
    let closest = |v: &[(f64, usize)]| {
        let mut best = 2f64.powi(51);
        let mut m = None;
        for &(d, i) in v {
            if d < best {
                best = d;
                m = Some(i);
            }
        }
        m
    };
    if !inrange.is_empty() {
        closest(&inrange)
    } else if !norange.is_empty() {
        closest(&norange)
    } else if sizes.default.is_some() {
        sizes.default
    } else if askedsize == 0.0 {
        choose_size(sizes, 655360.0)
    } else {
        None
    }
}

/// CaseFolding.txt's common and full foldings, and UnicodeData.txt's letters
/// and numbers, for `sanitize_fontname`.
struct Folding {
    fold: BTreeMap<u32, Vec<u32>>,
    alnum: std::collections::BTreeSet<u32>,
}

fn folding() -> &'static Folding {
    static F: OnceLock<Folding> = OnceLock::new();
    F.get_or_init(|| {
        let read = |n: &str| {
            crate::kpse::locate(n)
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default()
        };
        let mut fold = BTreeMap::new();
        for line in read("CaseFolding.txt").lines() {
            let f: Vec<&str> = line.split(';').map(str::trim).collect();
            if f.len() < 3 || !(f[1] == "C" || f[1] == "F") {
                continue;
            }
            let Ok(c) = u32::from_str_radix(f[0], 16) else {
                continue;
            };
            let m: Vec<u32> = f[2]
                .split(' ')
                .filter_map(|x| u32::from_str_radix(x, 16).ok())
                .collect();
            fold.insert(c, m);
        }
        let mut alnum = std::collections::BTreeSet::new();
        for line in read("UnicodeData.txt").lines() {
            let f: Vec<&str> = line.splitn(4, ';').collect();
            if f.len() >= 3 && (f[2].starts_with('L') || f[2].starts_with('N')) {
                if let Ok(c) = u32::from_str_radix(f[0], 16) {
                    alnum.insert(c);
                }
            }
        }
        Folding { fold, alnum }
    })
}

/// `sanitize_fontname` (:293-300): case folded, letters and digits only.
pub fn sanitize_fontname(s: &str) -> String {
    let f = folding();
    let mut out = String::new();
    for c in s.chars() {
        let mapped: Vec<u32> = f
            .fold
            .get(&(c as u32))
            .cloned()
            .unwrap_or_else(|| vec![c as u32]);
        for m in mapped {
            let keep = f.alnum.contains(&m)
                || (0x3400..=0x3DB5).contains(&m)
                || (0x4E00..=0x9FEF).contains(&m)
                || (0xAC00..=0xD7A3).contains(&m);
            if keep {
                if let Some(ch) = char::from_u32(m) {
                    out.push(ch);
                }
            }
        }
    }
    out
}

/// A request, as `fontreq::analyze` reads it, to the file it names and its
/// subfont: `resolve_file`, `resolve_path`, `resolve_name`, `resolve_anon`
/// and `resolve_kpse` (luaotfload-resolvers.lua:71-242).
pub fn resolve(
    spec: &crate::fontreq::Specification,
    size: Option<i64>,
) -> Option<(PathBuf, Option<u32>)> {
    use crate::fontreq::Lookup;
    let name = spec.name.as_str();
    let file = |name: &str| -> Option<(PathBuf, Option<u32>)> {
        // lookup_font_file (:749-780)
        if let Some(found) = Db::get().and_then(|db| db.lookup_filename(name)) {
            if Path::new(found).is_file() {
                return Some((PathBuf::from(found), None));
            }
        }
        if Path::new(name).extension().is_some() {
            if let Some(p) = crate::kpse::locate(name) {
                return Some((p, None));
            }
        }
        for metric in ["tfm", "ofm"] {
            if let Some(p) = crate::kpse::locate(&format!("{name}.{metric}")) {
                return Some((p, None));
            }
        }
        None
    };
    let byname = |name: &str| -> Option<(PathBuf, Option<u32>)> {
        let db = Db::get()?;
        // the `cached` resolver: a remembered answer first
        let key = hash_request(spec, size);
        if let Some((base, sub)) = db.lookups.get(&key) {
            if db.verify_font_file(base) {
                // the cache keeps what the resolver returned: a full path
                return Some((PathBuf::from(base), *sub));
            }
        }
        db.lookup_font_name(name, spec.style.as_deref(), spec.optsize, size)
    };
    let found = match spec.lookup {
        Lookup::File => file(name),
        Lookup::Path => match Path::new(name).is_file() {
            true => Some((PathBuf::from(name), None)),
            false => file(name),
        },
        Lookup::Name => byname(name).or_else(|| file(name)),
        Lookup::Kpse => {
            let ext = Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let direct = (!ext.is_empty())
                .then(|| crate::kpse::locate(name))
                .flatten();
            direct
                .or_else(|| {
                    ["otf", "ttf", "pfb", "lua", "afm"]
                        .iter()
                        .find_map(|t| crate::kpse::locate(&format!("{name}.{t}")))
                })
                .map(|p| (p, None))
        }
        // anon: tex, path, name
        _ => {
            let tex = ["tfm", "ofm"].iter().find_map(|f| {
                let n = if name.ends_with(&format!(".{f}")) {
                    name.to_string()
                } else {
                    format!("{name}.{f}")
                };
                crate::kpse::locate(&n)
            });
            tex.map(|p| (p, None))
                .or_else(|| {
                    Path::new(&spec.specification)
                        .is_file()
                        .then(|| (PathBuf::from(&spec.specification), None))
                })
                .or_else(|| byname(name).or_else(|| file(name)))
        }
    }?;
    // a subfont given in the request wins over none
    let sub = found.1.or(match &spec.sub {
        Some(crate::fontreq::SubFont::Index(n)) => Some(*n),
        _ => None,
    });
    Some((found.0, sub))
}

/// `hash_request` (:870-889): the request's specification, style, sub,
/// optsize and size joined by `#`, the absent ones left out.
fn hash_request(spec: &crate::fontreq::Specification, size: Option<i64>) -> String {
    let mut key = vec![spec.specification.clone()];
    if let Some(s) = &spec.style {
        key.push(s.clone());
    }
    match &spec.sub {
        Some(crate::fontreq::SubFont::Index(n)) => key.push(n.to_string()),
        Some(crate::fontreq::SubFont::Name(n)) => key.push(n.clone()),
        None => {}
    }
    if let Some(o) = spec.optsize {
        key.push(fmt_lua_number(o));
    }
    if let Some(s) = size {
        key.push(s.to_string());
    }
    key.join("#")
}

/// A number as Lua's `tostring` prints it in a concat.
fn fmt_lua_number(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}
