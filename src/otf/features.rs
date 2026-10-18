//! The features a node-mode font is processed with, from its request:
//! luaotfload's `apply_default_features` (luaotfload-features.lua:240-305)
//! with its configured defaults (luaotfload-configuration.lua:109-273), then
//! the fontloader's `otffeatures.normalize` (font-ott.lua, merged as
//! fontloader-2023-12-28.lua:9594-9644) and the defaults of the features
//! processing reads (`checkmarks`, `spacekern`: font-ots.lua:4311-4318,
//! 4560-4567; `normalize`: luaotfload-features.lua:868-892).
//!
//! A feature whose value comes out false is left out: the processor only
//! asks whether one is enabled (`enabled[kind]`).

use std::collections::BTreeMap;

use super::ots::FeatureValue;
use super::ott::{LANGUAGES, SCRIPTS};
use crate::shape::FeatureValue as Raw;

/// The node-mode feature set of one font.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeFeatures {
    pub features: BTreeMap<String, FeatureValue>,
    pub script: String,
    pub language: String,
}

/// luaotfload-configuration.lua:109-117.
const BASE_FEATURES: [&str; 15] = [
    "abvm", "blwm", "ccmp", "locl", "mark", "mkmk", "rlig", "calt", "clig", "curs", "dist", "kern",
    "liga", "rclt", "itlc",
];

/// The script presets of luaotfload-configuration.lua:118-134 and the
/// scripts `default_features` gives them (:251-272). `thai` names a preset
/// that does not exist, so Thai and Lao fall back on `dflt` like any script
/// without one.
fn preset(script: &str) -> &'static [&'static str] {
    match script {
        "arab" | "syrc" | "mong" | "nko" => &[
            "isol", "fina", "fin2", "fin3", "medi", "med2", "init", "cswh", "mset",
        ],
        "deva" | "beng" | "guru" | "gujr" | "orya" | "taml" | "telu" | "knda" | "mlym" | "sinh" => {
            &[
                "init", "nukt", "akhn", "rphf", "blwf", "half", "pstf", "vatu", "pres", "blws",
                "abvs", "psts", "haln",
            ]
        }
        "khmr" | "tibt" => &[
            "pref", "blwf", "abvf", "pstf", "pres", "blws", "abvs", "psts",
        ],
        "hang" => &["ccmp", "ljmo", "vjmo", "tjmo"],
        _ => &[],
    }
}

/// `stringgsub(lower(v), "[^a-z0-9]", "")`.
fn squash(v: &str) -> String {
    v.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// `rawget(verbose, v) or (tags[v] and v) or "dflt"`. The verbose table is
/// the tags table swapped (`swapped`, luaotfload-features.lua:200-206); where
/// two tags share a squashed name the later one is taken here, which a Lua
/// hash traversal does not promise.
fn resolve(value: &str, tags: &[(&str, &str)]) -> String {
    let v = squash(value);
    if let Some((tag, _)) = tags.iter().rev().find(|(_, name)| squash(name) == v) {
        return tag.to_string();
    }
    if tags.iter().any(|(tag, _)| *tag == v) {
        return v;
    }
    "dflt".to_string()
}

/// `is_boolean` (l-boolean.lua): the words a string feature value means a
/// boolean by.
fn is_boolean(s: &str) -> Option<bool> {
    match s {
        "true" | "yes" | "on" | "t" => Some(true),
        "false" | "no" | "off" | "f" => Some(false),
        _ => None,
    }
}

/// The features `raw` (a request's `features.raw`) gives a node-mode font.
pub fn node_features(raw: &BTreeMap<String, Raw>) -> NodeFeatures {
    // apply_default_features: "true"/"false" strings become booleans
    let mut spec: BTreeMap<String, Raw> = BTreeMap::new();
    for (k, v) in raw {
        let v = match v {
            Raw::Text(t) if t.to_lowercase() == "true" => Raw::Bool(true),
            Raw::Text(t) if t.to_lowercase() == "false" => Raw::Bool(false),
            v => v.clone(),
        };
        spec.insert(k.clone(), v);
    }
    spec.remove("");
    let text = |v: Option<&Raw>| match v {
        Some(Raw::Text(t)) => Some(t.clone()),
        _ => None,
    };
    let language =
        text(spec.get("language")).map_or("dflt".to_string(), |l| resolve(&l, LANGUAGES));
    let script = text(spec.get("script")).map_or("dflt".to_string(), |s| resolve(&s, SCRIPTS));
    spec.insert("language".into(), Raw::Text(language.clone()));
    spec.insert("script".into(), Raw::Text(script.clone()));
    for f in BASE_FEATURES.iter().chain(preset(&script)) {
        spec.entry(f.to_string()).or_insert(Raw::Bool(true));
    }
    spec.entry("mode".into())
        .or_insert(Raw::Text("node".into()));

    // otffeatures.normalize: keys lowercased; a number stays one, a string
    // is a boolean word or else lowercased
    let mut features = BTreeMap::new();
    for (k, v) in &spec {
        let k = k.to_lowercase();
        if k == "language" || k == "script" {
            continue;
        }
        let v = match v {
            Raw::Bool(b) => b.then_some(FeatureValue::True),
            Raw::Number(n) => Some(FeatureValue::Number(*n)),
            Raw::Text(t) => match t.parse::<f64>() {
                Ok(n) => Some(FeatureValue::Number(n)),
                Err(_) => match is_boolean(t) {
                    Some(b) => b.then_some(FeatureValue::True),
                    None => Some(FeatureValue::Text(t.to_lowercase())),
                },
            },
        };
        match v {
            Some(v) => {
                features.insert(k, v);
            }
            None => {
                features.remove(&k);
            }
        }
    }
    // checkedfeatures: the defaults of the features processing reads, where
    // the request says nothing (`normalize`: luaotfload-features.lua:868-871;
    // `szss`: luaotfload-szss.lua:64-77; `amtra`: luaotfload-arabic.lua:126-143;
    // `analyze`: font-ota.lua:239-250; `notdef`, `invisible`:
    // luaotfload-notdef.lua:155-165, 363-371)
    let defaults = [
        ("checkmarks", FeatureValue::True),
        ("spacekern", FeatureValue::True),
        ("normalize", FeatureValue::Text("nfc".into())),
        ("szss", FeatureValue::Text("auto".into())),
        ("amtra", FeatureValue::Text("auto".into())),
        ("analyze", FeatureValue::True),
        ("features", FeatureValue::True),
        ("notdef", FeatureValue::Number(1.0)),
        ("invisible", FeatureValue::True),
    ];
    for (f, v) in defaults {
        let said = spec.keys().any(|k| k.to_lowercase() == f);
        if !said {
            features.insert(f.to_string(), v);
        }
    }
    NodeFeatures {
        features,
        script,
        language,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(pairs: &[(&str, Raw)]) -> BTreeMap<String, Raw> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn defaults_follow_the_script() {
        let latin = node_features(&raw(&[]));
        assert_eq!(
            (latin.script.as_str(), latin.language.as_str()),
            ("dflt", "dflt")
        );
        for f in [
            "kern",
            "liga",
            "mark",
            "mkmk",
            "itlc",
            "spacekern",
            "checkmarks",
        ] {
            assert!(latin.features.contains_key(f), "{f}");
        }
        assert!(!latin.features.contains_key("init"));
        let arabic = node_features(&raw(&[("script", Raw::Text("Arabic".into()))]));
        assert_eq!(arabic.script, "arab");
        assert!(arabic.features.contains_key("init") && arabic.features.contains_key("kern"));
        // a script with a nonexistent preset falls back on the base set
        let thai = node_features(&raw(&[("script", Raw::Text("thai".into()))]));
        assert_eq!(thai.script, "thai");
        assert!(!thai.features.contains_key("init") && thai.features.contains_key("liga"));
    }

    #[test]
    fn request_values_override_and_normalise() {
        let f = node_features(&raw(&[
            ("kern", Raw::Bool(false)),
            ("liga", Raw::Text("no".into())),
            ("salt", Raw::Text("2".into())),
            ("ss01", Raw::Text("Yes".into())),
            ("language", Raw::Text("DEU".into())),
            ("spacekern", Raw::Bool(false)),
        ]));
        assert!(!f.features.contains_key("kern"));
        assert!(!f.features.contains_key("liga"));
        assert!(!f.features.contains_key("spacekern"));
        assert_eq!(f.features.get("salt"), Some(&FeatureValue::Number(2.0)));
        // `is_boolean` sees the value as written, so "Yes" stays a word
        assert_eq!(
            f.features.get("ss01"),
            Some(&FeatureValue::Text("yes".into()))
        );
        assert_eq!(f.language, "deu");
        assert_eq!(
            node_features(&raw(&[("language", Raw::Text("german".into()))])).language,
            "deu"
        );
        assert_eq!(
            node_features(&raw(&[("language", Raw::Text("xx-yy".into()))])).language,
            "dflt"
        );
    }
}
