//! Photo details (location, credit, caption) that Windows keeps for Desktop
//! Spotlight images, read from the registry.
//!
//! Windows stores them as JSON under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\DesktopSpotlight`. Each entry
//! ("creative") names its image file in `ad.landscapeImage.asset`, which is how
//! an image is matched to its details. The format isn't documented, so anything
//! unexpected is skipped rather than guessed at. The list also includes the
//! wallpapers built into Windows, but those are never scanned, so never saved.

use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Info {
    /// "A pier in Corinthia, Greece"
    pub location: Option<String>,
    /// "© ilias beros/ 500px Prime / Getty Images"
    pub credit: Option<String>,
    /// Spotlight's caption: "Peering over smooth seas"
    pub title: Option<String>,
    /// The paragraph of trivia shown on the desktop.
    pub description: Option<String>,
    /// The "Learn more" link on Bing.
    pub link: Option<String>,
}

/// Image path → details, for every creative found in the registry.
pub struct Catalog(HashMap<String, Info>);

impl Catalog {
    pub fn load() -> Self {
        let mut map = HashMap::new();
        for text in registry_strings() {
            if let Ok(json) = serde_json::from_str::<Value>(&text) {
                collect(&json, &mut map);
            }
        }
        Catalog(map)
    }

    #[cfg(test)]
    pub fn from_json(text: &str) -> Self {
        let mut map = HashMap::new();
        collect(&serde_json::from_str(text).unwrap(), &mut map);
        Catalog(map)
    }

    pub fn get(&self, path: &Path) -> Option<&Info> {
        self.0.get(&key(path))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// Paths compare case-insensitively on Windows, and may use either slash.
fn key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// Finds creatives anywhere in a JSON value, since where Windows nests them
/// may change between builds.
fn collect(value: &Value, out: &mut HashMap<String, Info>) {
    match value {
        Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
        Value::Object(obj) => {
            if let Some(ad) = obj.get("ad") {
                if let Some(asset) = ad.pointer("/landscapeImage/asset").and_then(Value::as_str) {
                    out.insert(key(Path::new(&expand_env(asset))), parse_ad(ad));
                    return;
                }
            }
            obj.values().for_each(|v| collect(v, out));
        }
        _ => {}
    }
}

pub fn parse_ad(ad: &Value) -> Info {
    let text = |name: &str| {
        ad.get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    // "A pier in Corinthia, Greece\r\n© ilias beros / Getty Images\r\nRight-click to learn more"
    let hover = text("iconHoverText").unwrap_or_default();
    let mut lines = hover.lines().map(str::trim).filter(|l| !l.is_empty());
    let location = lines.next().map(String::from);
    let credit = text("copyright").or_else(|| lines.next().map(String::from));
    Info {
        location,
        credit,
        title: text("title"),
        description: text("description"),
        link: text("ctaUri"),
    }
}

/// Expands `%Name%` environment variables, in case a path uses them.
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => match std::env::var(&after[..end]) {
                Ok(value) => {
                    out.push_str(&value);
                    rest = &after[end + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Every text value under the DesktopSpotlight registry key and its subkeys.
#[cfg(windows)]
fn registry_strings() -> Vec<String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    fn walk(key: &RegKey, depth: u32, out: &mut Vec<String>) {
        for (name, _) in key.enum_values().flatten() {
            if let Ok(text) = key.get_value::<String, _>(&name) {
                out.push(text);
            }
        }
        if depth == 0 {
            return;
        }
        for name in key.enum_keys().flatten() {
            if let Ok(sub) = key.open_subkey(&name) {
                walk(&sub, depth - 1, out);
            }
        }
    }

    let mut out = Vec::new();
    let root = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = root.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\DesktopSpotlight") {
        walk(&key, 3, &mut out);
    }
    out
}

#[cfg(not(windows))]
fn registry_strings() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/data/default_creatives.json");

    #[test]
    fn reads_default_creatives() {
        let catalog = Catalog::from_json(SAMPLE);
        assert_eq!(catalog.len(), 2);

        let path = Path::new(
            r"c:\windows\SystemApps\MicrosoftWindows.Client.CBS_cw5n1h2txyewy\DesktopSpotlight\Assets\Images\IMAGE_2.jpg",
        );
        let info = catalog.get(path).expect("matched case-insensitively");
        assert_eq!(info.location.as_deref(), Some("A pier in Corinthia, Greece"));
        assert_eq!(info.credit.as_deref(), Some("ilias beros/ 500px Prime / Getty Images"));
        assert_eq!(info.title.as_deref(), Some("Peering over smooth seas"));
        assert!(info.description.as_deref().unwrap().starts_with("Corinthia is a regional unit"));
        assert!(info.link.as_deref().unwrap().contains("spotlightId=PierSeascapeKorinthia"));
    }

    #[test]
    fn credit_falls_back_to_hover_text() {
        let json = r#"[{"ad":{"landscapeImage":{"asset":"C:\\a.jpg"},
            "iconHoverText":"Ronda, Spain\r\n© Someone / Getty Images\r\nRight-click to learn more"}}]"#;
        let catalog = Catalog::from_json(json);
        let info = catalog.get(Path::new(r"C:\a.jpg")).unwrap();
        assert_eq!(info.location.as_deref(), Some("Ronda, Spain"));
        assert_eq!(info.credit.as_deref(), Some("© Someone / Getty Images"));
        assert_eq!(info.title, None);
    }

    #[test]
    fn ignores_unrelated_json() {
        let catalog = Catalog::from_json(r#"{"Version":0,"RetrieveIrisContentSuccess":false}"#);
        assert_eq!(catalog.len(), 0);
    }

    #[test]
    fn expands_environment_variables() {
        std::env::set_var("SPOTLIGHT_TEST_ROOT", r"C:\Windows");
        assert_eq!(expand_env(r"%SPOTLIGHT_TEST_ROOT%\x.jpg"), r"C:\Windows\x.jpg");
        assert_eq!(expand_env("100% sure"), "100% sure");
        assert_eq!(expand_env("%NO_SUCH_VAR_X%\\a"), "%NO_SUCH_VAR_X%\\a");
    }
}
