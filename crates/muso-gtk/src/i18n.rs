//! Translations. Strings in the code are English message ids; the
//! translations are gettext `.po` files in `po/` (edit them with any gettext
//! tool), built into the binary so the tarball needs no locale directory.
//! The language follows the usual `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` and
//! `LANG` variables.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;

use gtk::glib;

include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

/// `msgid` in the user's language, or as is when there's no translation.
pub fn tr(msgid: &str) -> String {
    catalog()
        .get(msgid)
        .cloned()
        .unwrap_or_else(|| msgid.to_owned())
}

/// [`tr`] with each `{}` in the message replaced by the next argument.
pub fn tr_args(msgid: &str, args: &[&dyn Display]) -> String {
    let message = tr(msgid);
    let mut out = String::new();
    let mut args = args.iter();
    let mut parts = message.split("{}");
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    for part in parts {
        if let Some(arg) = args.next() {
            out.push_str(&arg.to_string());
        }
        out.push_str(part);
    }
    out
}

fn catalog() -> &'static HashMap<String, String> {
    static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let names = glib::language_names();
        names
            .iter()
            .find_map(|name| CATALOGS.iter().find(|(lang, _)| *lang == name.as_str()))
            .map(|(_, po)| parse_po(po))
            .unwrap_or_default()
    })
}

/// The translated entries of a `.po` file: `msgid` to `msgstr`, leaving out
/// the header, fuzzy and untranslated entries.
fn parse_po(po: &str) -> HashMap<String, String> {
    #[derive(PartialEq)]
    enum Field {
        None,
        Context,
        Id,
        Str,
    }
    let mut map = HashMap::new();
    let (mut context, mut id, mut text) = (String::new(), String::new(), String::new());
    let mut field = Field::None;
    let mut fuzzy = false;
    let mut flush = |context: &mut String, id: &mut String, text: &mut String, fuzzy: bool| {
        if !id.is_empty() && !text.is_empty() && !fuzzy {
            let key = if context.is_empty() {
                std::mem::take(id)
            } else {
                format!("{context}\u{4}{id}")
            };
            map.insert(key, std::mem::take(text));
        }
        context.clear();
        id.clear();
        text.clear();
    };
    for line in po.lines() {
        let line = line.trim();
        if line.starts_with("#,") {
            if field == Field::Str {
                flush(&mut context, &mut id, &mut text, fuzzy);
                field = Field::None;
            }
            fuzzy = line.contains("fuzzy");
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (target, rest) = if let Some(rest) = line.strip_prefix("msgctxt ") {
            if field == Field::Str {
                flush(&mut context, &mut id, &mut text, fuzzy);
                fuzzy = false;
            }
            field = Field::Context;
            (&mut context, rest)
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            if field == Field::Str {
                flush(&mut context, &mut id, &mut text, fuzzy);
                fuzzy = false;
            }
            field = Field::Id;
            (&mut id, rest)
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            field = Field::Str;
            (&mut text, rest)
        } else if line.starts_with('"') {
            match field {
                Field::Context => (&mut context, line),
                Field::Id => (&mut id, line),
                Field::Str => (&mut text, line),
                Field::None => continue,
            }
        } else {
            continue;
        };
        target.push_str(&unquote(rest));
    }
    flush(&mut context, &mut id, &mut text, fuzzy);
    map
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_po_entries() {
        let po = r#"
msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\n"

#: src/app.rs
msgid "Volume: {}%"
msgstr "Громкость: {}%"

msgid "Drop music here\n"
"or press O to open"
msgstr ""
"Перетащите музыку сюда\n"
"или нажмите O"

#, fuzzy
msgid "Paused"
msgstr "Пауза"

msgid "Quit"
msgstr ""
"#;
        let map = parse_po(po);
        assert_eq!(map.len(), 2, "{map:?}");
        assert_eq!(map["Volume: {}%"], "Громкость: {}%");
        assert_eq!(
            map["Drop music here\nor press O to open"],
            "Перетащите музыку сюда\nили нажмите O"
        );
    }

    /// Every string the code passes to `tr` has a translation in every
    /// catalog, and the catalogs have nothing left over.
    #[test]
    fn catalogs_match_the_code() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut used = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(src).unwrap().flatten() {
            if entry.file_name() == "i18n.rs" {
                continue;
            }
            let code = std::fs::read_to_string(entry.path()).unwrap();
            for call in ["tr(\"", "tr_args(\""] {
                let mut rest = code.as_str();
                while let Some(at) = rest.find(call) {
                    let whole_word = !rest[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_');
                    let piece = &rest[at + call.len()..];
                    // No escaped quotes in muso's messages.
                    let end = piece.find('"').unwrap();
                    if whole_word {
                        used.insert(unquote(&piece[..end]));
                    }
                    rest = &piece[end..];
                }
            }
        }
        for (lang, po) in CATALOGS {
            let map = parse_po(po);
            let missing: Vec<_> = used.iter().filter(|m| !map.contains_key(*m)).collect();
            assert!(missing.is_empty(), "{lang}.po misses {missing:?}");
            let unused: Vec<_> = map.keys().filter(|m| !used.contains(*m)).collect();
            assert!(unused.is_empty(), "{lang}.po has unused {unused:?}");
        }
    }

    #[test]
    fn arguments_fill_placeholders_in_order() {
        assert_eq!(tr_args("{} of {}", &[&1, &"two"]), "1 of two");
    }
}
