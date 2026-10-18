//! texrs's resolution of font requests to files against luaotfload's.
//!
//! `scripts/node-names-freeze.sh` loads each request of
//! `tests/data/node_names.txt` with luahbtex at 10pt and records the file
//! luaotfload resolved it to: `file:` and anonymous requests, and `name:`
//! requests through luaotfload's font names database. This resolves the
//! same requests with `texrs::otf::names` against the same database and
//! requires the same files. The database is luaotfload's on this machine, so
//! a machine without one (or without TeX Live) skips.

#[test]
fn requests_resolve_as_luaotfload_resolves_them() {
    if texrs::otf::names::Db::get().is_none() {
        eprintln!("skipping: no luaotfload font names database here");
        return;
    }
    let mut failures = Vec::new();
    for line in include_str!("data/node_names.txt").lines() {
        if line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let (request, expected) = (f[0], f[1]);
        let spec = texrs::fontreq::analyze(request).unwrap();
        let got = texrs::otf::names::resolve(&spec, Some(655_360))
            .map(|(p, _)| p.to_string_lossy().into_owned());
        if got.as_deref() != Some(expected) {
            failures.push(format!("{request}: luaotfload {expected}, texrs {got:?}"));
        }
        // a name request must come out the same without luaotfload's lookup
        // cache too: the database alone decides it
        if !request.ends_with(".otf") {
            let db = texrs::otf::names::Db::get().unwrap();
            let plain = db
                .lookup_font_name(
                    &spec.name,
                    spec.style.as_deref(),
                    spec.optsize,
                    Some(655_360),
                )
                .map(|(p, _)| p.to_string_lossy().into_owned());
            if plain.as_deref() != Some(expected) {
                failures.push(format!(
                    "{request} (uncached): luaotfload {expected}, texrs {plain:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
