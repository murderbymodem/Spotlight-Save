# Spotlight Saver

A small Windows app that copies the **Windows Spotlight** wallpapers already on
your PC into `Pictures\Wallpaper`, so you keep them after Windows swaps them out.
It doesn't need the Bing Wallpaper app and doesn't download anything.

## Where Windows keeps the images

| Spotlight on | Folder (under `%LOCALAPPDATA%\Packages`) |
|---|---|
| Desktop (Windows 11) | `MicrosoftWindows.Client.CBS_cw5n1h2txyewy\LocalCache\Microsoft\IrisService\*` |
| Lock screen | `Microsoft.Windows.ContentDeliveryManager_cw5n1h2txyewy\LocalState\Assets` |

The app scans these folders and skips icons, small images and portrait (phone)
versions. It never saves the same image twice, even if you rename the files or
edit their properties.

## Names and photo details

For Desktop Spotlight images, Windows keeps the details shown by **Learn about
this picture** in the registry, under
`HKCU\Software\Microsoft\Windows\CurrentVersion\DesktopSpotlight`. The app
names each file after the date it was saved and the photo's location, such as
`2026-11-22 A pier in Corinthia, Greece.jpg`, and writes the rest into the file's
properties (right-click → Properties → Details):

| Property | Contains |
|---|---|
| Title | The location |
| Subject | Spotlight's caption, e.g. "Peering over smooth seas" |
| Authors, Copyright | The photo credit |
| Comments | The description and the **Learn more** link on Bing |
| Tags | `Windows Spotlight` and the location |

Windows only lists details for the wallpapers it is currently rotating through,
so `--install` is the way to get them for every image. Images whose details are
gone (and lock screen images, which have none) are saved as
`<date> Windows Spotlight.jpg`.

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
