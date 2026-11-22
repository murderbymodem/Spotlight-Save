# Spotlight Saver

A small Windows app that copies the **Windows Spotlight** wallpapers already on
your PC into `Pictures\Wallpaper`, so you keep them after Windows swaps them out.
It doesn't need the Bing Wallpaper app and doesn't download anything.

## Where Windows keeps the images

| Spotlight on | Folder (under `%LOCALAPPDATA%\Packages`) |
|---|---|
| Desktop (Windows 11) | `MicrosoftWindows.Client.CBS_cw5n1h2txyewy\LocalCache\Microsoft\IrisService\*` |
| Lock screen | `Microsoft.Windows.ContentDeliveryManager_cw5n1h2txyewy\LocalState\Assets` |

The app scans both folders. It skips icons, small images and portrait (phone)
versions, then saves each image as `Spotlight_<date>_<hash>.jpg`. It never saves
the same image twice, even if you rename the files.

## Use

```
spotlight-saver.exe              Save new wallpapers and show a summary
spotlight-saver.exe --quiet      Save new wallpapers without any window
spotlight-saver.exe --install    Also save them every hour (Task Scheduler)
spotlight-saver.exe --uninstall  Remove the hourly task
```

Windows only keeps a few Spotlight images at a time, so use `--install` to catch
each one. Move the exe to a permanent folder first, because the scheduled task
runs it from wherever it is when you install it.

## Build

On Windows, with [Rust](https://rustup.rs) installed:

```
cargo build --release
```

The exe is `target\release\spotlight-saver.exe`.

Publishing a GitHub release builds the exe automatically and attaches it to the
release (see `.github/workflows/release.yml`). To add the exe to a release that
already exists, run the **Release** workflow from the Actions tab and enter its
tag.
