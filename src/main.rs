//! Spotlight Saver
//!
//! Copies the Windows Spotlight images that Windows has already downloaded
//! into `Pictures\Wallpaper`, so you keep them after Windows rotates them out.
//!
//! Usage:
//!   spotlight-saver.exe              Save new wallpapers and show a summary.
//!   spotlight-saver.exe --quiet      Save new wallpapers without any window.
//!   spotlight-saver.exe --install    Run quietly every hour (Task Scheduler).
//!   spotlight-saver.exe --uninstall  Remove the scheduled task.

// No console window: this is a GUI-subsystem app that only shows a message box.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const TASK_NAME: &str = "Spotlight Saver";

/// Files smaller than this are icons, logos and ad tiles, not wallpapers.
const MIN_BYTES: u64 = 100 * 1024;

/// Spotlight wallpapers are at least this wide (usually 1920 or 3840).
const MIN_WIDTH: usize = 1280;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a.eq_ignore_ascii_case(flag));

    if has("--install") {
        report(install_task(), "Spotlight Saver will now save new wallpapers every hour.");
    } else if has("--uninstall") {
        report(uninstall_task(), "The Spotlight Saver scheduled task was removed.");
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
    let dest = pictures.join("Wallpaper");
    fs::create_dir_all(&dest)?;

    let known = existing_hashes(&dest);
    let mut seen = HashSet::new();
    let mut saved = 0;
    let mut already_had = 0;

    for file in candidate_files() {
        let Some((bytes, ext)) = read_wallpaper(&file) else { continue };
        let hash = fnv1a(&bytes);
        if !seen.insert(hash) {
            continue; // Same image found in two places during this run.
        }
        if known.contains(&hash) {
            already_had += 1;
            continue;
        }

        let date = fs::metadata(&file)
            .and_then(|m| m.modified())
            .map(date_string)
            .unwrap_or_else(|_| date_string(SystemTime::now()));
        let target = dest.join(format!("Spotlight_{date}_{hash:016x}.{ext}"));
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

/// Hashes of images already in the destination, read from their file names, so
/// re-running never makes duplicates. (Renamed files are hashed from their contents.)
fn existing_hashes(dest: &Path) -> HashSet<u64> {
    let mut hashes = HashSet::new();
    let Ok(entries) = fs::read_dir(dest) else { return hashes };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(hash) = hash_from_name(&path) {
            hashes.insert(hash);
        } else if path.is_file() {
            if let Ok(bytes) = fs::read(&path) {
                hashes.insert(fnv1a(&bytes));
            }
        }
    }
    hashes
}

fn hash_from_name(path: &Path) -> Option<u64> {
    let stem = path.file_stem()?.to_str()?;
    let rest = stem.strip_prefix("Spotlight_")?;
    let hex = rest.rsplit('_').next()?;
    if hex.len() != 16 {
        return None;
    }
    u64::from_str_radix(hex, 16).ok()
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
    schtasks(&["/Create", "/F", "/TN", TASK_NAME, "/SC", "HOURLY", "/TR", &action])?;
    // Save the current wallpapers right away too.
    save_wallpapers().map(|_| ())
}

fn uninstall_task() -> io::Result<()> {
    schtasks(&["/Delete", "/F", "/TN", TASK_NAME])
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
        let title = wide("Spotlight Saver");
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
    fn names_round_trip() {
        let hash = fnv1a(b"hello");
        let name = format!("Spotlight_2025-11-21_{hash:016x}.jpg");
        assert_eq!(hash_from_name(Path::new(&name)), Some(hash));
        assert_eq!(hash_from_name(Path::new("holiday.jpg")), None);
    }

    #[test]
    fn detects_formats() {
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(image_extension(b"GIF89a"), None);
    }
}
