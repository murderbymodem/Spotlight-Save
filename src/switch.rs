//! Switches the Desktop Spotlight wallpaper to the next or previous picture.
//!
//! Windows keeps the downloaded pictures as a list in
//! `HKCU\...\DesktopSpotlight\Creatives` (value `Creatives`) and the current
//! one's position in `ImageIndex`. Changing `ImageIndex` makes Windows show that
//! picture itself, so Spotlight stays in charge of the background. Setting the
//! wallpaper directly could turn Spotlight off, so the app never does that.

use std::io;

#[derive(Clone, Copy)]
pub enum Direction {
    Next,
    Previous,
}

/// The position to move to in a list of `count` pictures, wrapping around at
/// either end. None if there's nothing to switch to.
fn step(index: u32, count: usize, direction: Direction) -> Option<u32> {
    let count = u32::try_from(count).ok()?;
    if count < 2 {
        return None;
    }
    let index = index % count;
    Some(match direction {
        Direction::Next => (index + 1) % count,
        Direction::Previous => (index + count - 1) % count,
    })
}

/// Switches the wallpaper and returns a message saying what it switched to.
#[cfg(windows)]
pub fn switch(direction: Direction) -> io::Result<String> {
    use crate::info::parse_ad;
    use serde_json::Value;
    use std::time::{Duration, Instant};
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    const CREATIVES: &str = r"Software\Microsoft\Windows\CurrentVersion\DesktopSpotlight\Creatives";
    let not_found = || {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Windows Spotlight hasn't downloaded any desktop pictures yet. \
             Turn it on in Settings > Personalization > Background.",
        )
    };

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey_with_flags(CREATIVES, KEY_READ | KEY_WRITE).map_err(|_| not_found())?;
    let list: String = key.get_value("Creatives").map_err(|_| not_found())?;
    let creatives = match serde_json::from_str::<Value>(&list) {
        Ok(Value::Array(items)) => items,
        _ => return Err(not_found()),
    };
    let index: u32 = key.get_value("ImageIndex").unwrap_or(0);
    let Some(target) = step(index, creatives.len(), direction) else {
        return Err(io::Error::other("Windows Spotlight only has one picture right now, so there's nothing to switch to."));
    };
    key.set_value("ImageIndex", &target)?;

    let ad = creatives[target as usize].get("ad").cloned().unwrap_or(Value::Null);
    let name = parse_ad(&ad).location.unwrap_or_else(|| format!("picture {} of {}", target + 1, creatives.len()));
    let asset = ad.pointer("/landscapeImage/asset").and_then(Value::as_str).unwrap_or_default().to_lowercase();

    // Windows applies the change on its own; wait briefly to confirm it did.
    let desktop = hkcu.open_subkey(r"Control Panel\Desktop")?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let current: String = desktop.get_value("WallPaper").unwrap_or_default();
        if !asset.is_empty() && current.to_lowercase() == asset {
            return Ok(format!("Switched to {name}."));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(format!("Asked Windows to switch to {name}, but it hasn't changed the wallpaper yet. It may take a moment."))
}

#[cfg(not(windows))]
pub fn switch(_direction: Direction) -> io::Result<String> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "Switching wallpapers only works on Windows."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_and_wraps() {
        assert_eq!(step(1, 4, Direction::Next), Some(2));
        assert_eq!(step(3, 4, Direction::Next), Some(0));
        assert_eq!(step(1, 4, Direction::Previous), Some(0));
        assert_eq!(step(0, 4, Direction::Previous), Some(3));
        // An out-of-range index (the list shrank) still lands inside it.
        assert_eq!(step(9, 4, Direction::Next), Some(2));
    }

    #[test]
    fn nothing_to_switch_to() {
        assert_eq!(step(0, 0, Direction::Next), None);
        assert_eq!(step(0, 1, Direction::Previous), None);
    }
}
