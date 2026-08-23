//! Language handling and translation loading for the internationalized index page.
//!
//! Translations live in `DATA_PATH/i18n/{en,ja,es,pt-BR,ko,ru}.json` (flat key -> value maps,
//! all files sharing the exact same key set). Values may contain `{0}`, `{1}`
//! positional placeholders that JavaScript fills at runtime via `t(key, args)`.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lang {
    En,
    Ja,
    Es,
    PtBr,
    Ko,
    Ru,
}

#[allow(dead_code)] // path/label/parsing wired up in Task 2
impl Lang {
    pub const ALL: [Lang; 6] = [Lang::En, Lang::Ja, Lang::Es, Lang::PtBr, Lang::Ko, Lang::Ru];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ja => "ja",
            Lang::Es => "es",
            Lang::PtBr => "pt-BR",
            Lang::Ko => "ko",
            Lang::Ru => "ru",
        }
    }

    pub fn path(self) -> &'static str {
        self.code()
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ja => "日本語",
            Lang::Es => "Español",
            Lang::PtBr => "Português (BR)",
            Lang::Ko => "한국어",
            Lang::Ru => "Русский",
        }
    }

    /// First *supported* tag's primary subtag, skipping unsupported tags and `*`;
    /// none supported -> En.
    pub fn from_accept_language(header: &str) -> Lang {
        for tag in header.split(',') {
            let subtag = tag.split(';').next().unwrap_or_default().trim();
            if subtag.is_empty() || subtag == "*" {
                continue;
            }
            let primary = subtag
                .split(['-', '_'])
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            match primary.as_str() {
                "ja" => return Lang::Ja,
                "es" => return Lang::Es,
                "pt" => return Lang::PtBr,
                "ko" => return Lang::Ko,
                "ru" => return Lang::Ru,
                "en" => return Lang::En,
                _ => continue,
            }
        }
        Lang::En
    }

    pub fn from_path(segment: &str) -> Option<Lang> {
        match segment {
            "en" => Some(Lang::En),
            "ja" => Some(Lang::Ja),
            "es" => Some(Lang::Es),
            "pt-BR" => Some(Lang::PtBr),
            "ko" => Some(Lang::Ko),
            "ru" => Some(Lang::Ru),
            _ => None,
        }
    }
}

/// Reads `i18n/{lang}.json` files under `data_path` for every [`Lang`] and
/// validates that all files share the exact same key set as `en.json`.
pub fn load_translations(data_path: &Path) -> Result<BTreeMap<Lang, BTreeMap<String, String>>> {
    let mut maps = BTreeMap::new();
    for lang in Lang::ALL {
        let path = data_path.join("i18n").join(format!("{}.json", lang.code()));
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let map: BTreeMap<String, String> = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        maps.insert(lang, map);
    }

    let en = &maps[&Lang::En];
    for (lang, map) in &maps {
        for key in en.keys() {
            if !map.contains_key(key) {
                return Err(anyhow!("i18n key '{key}' is missing from {}.json", lang.code()));
            }
        }
        for key in map.keys() {
            if !en.contains_key(key) {
                return Err(anyhow!(
                    "i18n key '{key}' is present only in {}.json",
                    lang.code()
                ));
            }
        }
    }
    Ok(maps)
}

/// Minimal HTML-escapes for values injected into HTML text and attributes.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
