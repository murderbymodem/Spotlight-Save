//! Spotlight Save
//!
//! Copies the Windows Spotlight images that Windows has already downloaded
//! into `Pictures\Spotlight`, so you keep them after Windows rotates them out.
//!
//! Usage:
//!   spotlight-save.exe              Save new wallpapers and show a summary.
//!   spotlight-save.exe --quiet      Save new wallpapers without any window.
//!   spotlight-save.exe --install    Run quietly every hour (Task Scheduler).
//!   spotlight-save.exe --uninstall  Remove the scheduled task.
//!   spotlight-save.exe --next       Switch to the next Spotlight wallpaper.
//!   spotlight-save.exe --previous   Switch to the previous Spotlight wallpaper.

// No console window: this is a GUI-subsystem app that only shows a message box.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod info;
mod jpeg;
mod switch;

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use info::{Catalog, Info};

const TASK_NAME: &str = "Spotlight Save";

/// The task's name before the app was renamed from Spotlight Saver. Installing
/// or uninstalling also removes it, so an old install doesn't keep running.
const OLD_TASK_NAME: &str = "Spotlight Saver";

/// Files smaller than this are icons, logos and ad tiles, not wallpapers.
const MIN_BYTES: u64 = 100 * 1024;

/// Spotlight wallpapers are at least this wide (usually 1920 or 3840).
const MIN_WIDTH: usize = 1280;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a.eq_ignore_ascii_case(flag));

    if has("--next") || has("--previous") {
        let direction = if has("--next") { switch::Direction::Next } else { switch::Direction::Previous };
        match switch::switch(direction) {
            Ok(message) if !has("--quiet") => ui::info(&message),
            Ok(_) => {}
            Err(e) if !has("--quiet") => ui::error(&format!("Could not switch the wallpaper:\n\n{e}")),
            Err(_) => std::process::exit(1),
        }
    } else if has("--install") {
        report(install_task(), "Spotlight Save will now save new wallpapers every hour.");
    } else if has("--uninstall") {
        report(uninstall_task(), "The Spotlight Save scheduled task was removed.");
    } else {
        let result = save_wallpapers();
        if has("--quiet") {
            if result.is_err() {
                std::process::exit(1);
            }
        } else {
            match result {
                Ok(summary) => ui::show_summary(&summary),
                Err(e) => ui::error(&format!("Could not save wallpapers:\n\n{e}")),
            }
        }
    }
}

fn report(result: io::Result<()>, success: &str) {
    match result {
        Ok(()) => ui::info(success),
        Err(e) => ui::error(&e.to_string()),
    }
}

pub struct Summary {
    pub saved: usize,
    pub already_had: usize,
    pub dest: PathBuf,
}

fn save_wallpapers() -> io::Result<Summary> {
    let pictures = dirs::picture_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Pictures folder not found"))?;
    let dest = pictures.join("Spotlight");
    fs::create_dir_all(&dest)?;

    let catalog = Catalog::load();
    let known = existing_hashes(&dest);
    let mut seen = HashSet::new();
    let mut saved = 0;
    let mut already_had = 0;
    let date = date_string(SystemTime::now());

    for file in candidate_files() {
        let Some((bytes, ext)) = read_wallpaper(&file) else { continue };
        let hash = fnv1a(jpeg::picture_data(&bytes));
        if !seen.insert(hash) {
            continue; // Same image found in two places during this run.
        }
        if known.contains(&hash) {
            already_had += 1;
            continue;
        }

        let info = catalog.get(&file);
        let bytes = match info {
            Some(info) => jpeg::with_info(&bytes, info),
            None => bytes,
        };
        let target = unique_path(&dest, &file_stem(&date, info), ext);
        fs::write(&target, &bytes)?;
        saved += 1;
    }

    Ok(Summary { saved, already_had, dest })
}

/// Every file in the folders where Windows keeps Spotlight images.
fn candidate_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Some(local) = dirs::data_local_dir() else { return files };
    let packages = local.join("Packages");

    // Desktop Spotlight (Windows 11): full-size JPEGs, one folder per image, under
    // Packages\MicrosoftWindows.Client.CBS_<id>\LocalCache\Microsoft\IrisService.
    // The package name has changed between Windows builds, so check them all.
    if let Ok(entries) = fs::read_dir(&packages) {
        for entry in entries.flatten() {
            let iris = entry.path().join("LocalCache").join("Microsoft").join("IrisService");
            collect_files(&iris, 4, &mut files);
        }
    }

    // Lock screen Spotlight (Windows 10 and 11): extensionless files mixed with icons.
    let assets = packages
        .join("Microsoft.Windows.ContentDeliveryManager_cw5n1h2txyewy")
        .join("LocalState")
        .join("Assets");
    collect_files(&assets, 0, &mut files);

    files
}

fn collect_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_file() {
            out.push(entry.path());
        } else if kind.is_dir() && depth > 0 {
            collect_files(&entry.path(), depth - 1, out);
        }
    }
}

/// Returns the file's bytes and extension if it is a landscape wallpaper-sized image.
fn read_wallpaper(path: &Path) -> Option<(Vec<u8>, &'static str)> {
    if fs::metadata(path).ok()?.len() < MIN_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let ext = image_extension(&bytes)?;
    let size = imagesize::blob_size(&bytes).ok()?;
    if size.width < MIN_WIDTH || size.width < size.height {
        return None; // Too small, or a portrait (phone) version.
    }
    Some((bytes, ext))
}

fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else {
        None
    }
}

/// Hashes of the images already in the destination, so re-running never makes
/// duplicates. Only the picture data is hashed, so renaming a file or editing
/// its properties doesn't make it look new.
fn existing_hashes(dest: &Path) -> HashSet<u64> {
    let mut hashes = HashSet::new();
    let Ok(entries) = fs::read_dir(dest) else { return hashes };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Ok(bytes) = fs::read(&path) {
                hashes.insert(fnv1a(jpeg::picture_data(&bytes)));
            }
        }
    }
    hashes
}

/// "2026-11-22 A pier in Corinthia, Greece", or "2026-11-22 Windows Spotlight"
/// when Windows no longer lists the image's details.
fn file_stem(date: &str, info: Option<&Info>) -> String {
    let location = info
        .and_then(|i| i.location.as_deref())
        .map(safe_file_name)
        .filter(|s| !s.is_empty());
    format!("{date} {}", location.as_deref().unwrap_or("Windows Spotlight"))
}

/// Removes characters Windows doesn't allow in file names and keeps it short.
fn safe_file_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | '|' => '-',
            '<' | '>' | ':' | '"' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let mut out: String = words.join(" ").chars().take(100).collect();
    // Windows drops trailing dots and spaces from names.
    while out.ends_with(['.', ' ']) {
        out.pop();
    }
    out
}

/// `dir\stem.ext`, or `dir\stem (2).ext` and so on if that name is taken.
fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    path
}

/// 64-bit FNV-1a: stable across builds, which is all deduplication needs.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// YYYY-MM-DD (UTC) for a timestamp, so the folder sorts by date.
fn date_string(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

fn install_task() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let action = format!("\"{}\" --quiet", exe.display());
    let _ = schtasks(&["/Delete", "/F", "/TN", OLD_TASK_NAME]);
    schtasks(&["/Create", "/F", "/TN", TASK_NAME, "/SC", "HOURLY", "/TR", &action])?;
    // Save the current wallpapers right away too.
    save_wallpapers().map(|_| ())
}

fn uninstall_task() -> io::Result<()> {
    let old = schtasks(&["/Delete", "/F", "/TN", OLD_TASK_NAME]);
    // Succeed if either task was there to remove.
    schtasks(&["/Delete", "/F", "/TN", TASK_NAME]).or(old)
}

fn schtasks(args: &[&str]) -> io::Result<()> {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output()?;
    if output.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(io::Error::other(format!("Task Scheduler said:\n\n{msg}")))
    }
}

#[cfg(windows)]
mod ui {
    use super::Summary;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_YESNO,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn message_box(text: &str, style: u32) -> i32 {
        let text = wide(text);
        let title = wide("Spotlight Save");
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), style) }
    }

    pub fn info(text: &str) {
        message_box(text, MB_OK | MB_ICONINFORMATION);
    }

    pub fn error(text: &str) {
        message_box(text, MB_OK | MB_ICONERROR);
    }

    pub fn show_summary(s: &Summary) {
        let text = format!(
            "Saved {} new wallpaper{} ({} already saved).\n\n{}\n\nOpen the folder?",
            s.saved,
            if s.saved == 1 { "" } else { "s" },
            s.already_had,
            s.dest.display()
        );
        if message_box(&text, MB_YESNO | MB_ICONINFORMATION) == IDYES {
            let _ = std::process::Command::new("explorer").arg(&s.dest).spawn();
        }
    }
}

/// Plain text output so the core logic can be built and tested off Windows.
#[cfg(not(windows))]
mod ui {
    use super::Summary;
    pub fn info(text: &str) {
        println!("{text}");
    }
    pub fn error(text: &str) {
        eprintln!("{text}");
    }
    pub fn show_summary(s: &Summary) {
        println!("Saved {} new, {} already saved, in {}", s.saved, s.already_had, s.dest.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(20_413), (2025, 11, 21));
    }

    #[test]
    fn file_names() {
        let info = Info { location: Some("A pier in Corinthia, Greece".into()), ..Info::default() };
        assert_eq!(file_stem("2026-11-22", Some(&info)), "2026-11-22 A pier in Corinthia, Greece");
        assert_eq!(file_stem("2026-11-22", None), "2026-11-22 Windows Spotlight");
        assert_eq!(file_stem("2026-11-22", Some(&Info::default())), "2026-11-22 Windows Spotlight");

        assert_eq!(safe_file_name("Rio: \"Sugarloaf\" / Brazil?"), "Rio Sugarloaf - Brazil");
        assert_eq!(safe_file_name("Washington, D.C."), "Washington, D.C");
        assert_eq!(safe_file_name(&"x".repeat(300)).len(), 100);
    }

    #[test]
    fn unique_names() {
        let dir = std::env::temp_dir().join(format!("spotlight-save-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "2026-11-22 Ronda, Spain", "jpg");
        assert_eq!(first.file_name().unwrap(), "2026-11-22 Ronda, Spain.jpg");
        fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "2026-11-22 Ronda, Spain", "jpg");
        assert_eq!(second.file_name().unwrap(), "2026-11-22 Ronda, Spain (2).jpg");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_formats() {
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(image_extension(b"GIF89a"), None);
    }
}
