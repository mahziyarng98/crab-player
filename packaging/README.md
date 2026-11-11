# Packaging

Every target installs the same things:

| File | Installed as |
| --- | --- |
| `target/release/crab-player` | `/usr/bin/crab-player` |
| `packaging/crab-player.desktop` | `/usr/share/applications/crab-player.desktop` |
| `assets/icons/hicolor/<size>x<size>/apps/crab-player.png` (16 to 512 px) | `/usr/share/icons/hicolor/<size>x<size>/apps/crab-player.png` |
| `LICENSE`, `README.md` | the distribution's licence and doc directories |

The desktop file name, its `StartupWMClass`, the Wayland app id and the X11
window class are all `crab-player`. Keep them identical: that is how
compositors and taskbars find the icon.

## Status

| Target | Status |
| --- | --- |
| User install (`scripts/install.sh`) | Installed and checked on Arch/Hyprland: runs from a clean `PATH`, desktop entry valid and listed for "Open with", opens a path with spaces through `%U`, Wayland app id `crab-player`. |
| Arch (`arch/`) | Built from the checkout with `makepkg` (117 tests in `check()`); package metadata and the packaged binary checked with the end-to-end render check. Not installed system-wide (needs root), not built from a release tarball, `namcap` not run (not installed). |
| Debian (`debian/`) | **Untested.** Never built. |
| Flatpak (`flatpak/`) | **Untested.** Never built; needs the mpv commit pinned and `cargo-sources.json` generated. |
| AppImage | Not provided: no AppImage tooling was available to build and verify one. |

## User-level install (no root)

```sh
scripts/install.sh        # builds the release binary, installs into ~/.local
scripts/uninstall.sh      # removes exactly what install.sh installed
```

Installs `~/.local/bin/crab-player`, `~/.local/share/applications/crab-player.desktop`
(with `Exec` pointing at the installed binary) and the icons under
`~/.local/share/icons/hicolor/`. Crab Player then appears in the launcher
and under "Open with"; default applications are **not** changed. `PREFIX`
overrides `~/.local`.

## Arch

Build from a checkout, without installing anything:

```sh
packaging/arch/build-local.sh            # package lands in target/arch/
sudo pacman -U target/arch/crab-player-*.pkg.tar.zst
```

For the AUR, set the real `url=` and `source=` in `PKGBUILD` and replace
`sha256sums=('SKIP')` with the tarball's checksum.

Dependencies: `mpv` (for `libmpv.so.2`), `ffmpeg` (Arch's build has every
decoder, including dav1d for AV1), `gcc-libs`, `glibc`, `libglvnd`,
`wayland`, `libxkbcommon`, `hicolor-icon-theme`. Optional:
`xdg-desktop-portal` and `xdg-desktop-portal-gtk` (file dialogs), `libva`
with `libva-mesa-driver` (AMD), `intel-media-driver` / `libva-intel-driver`
(Intel), `nvidia-utils` (NVDEC). Build: `cargo`.

## Debian / Ubuntu (untested)

`packaging/debian/` is a debhelper source package; copy it to the project
root as `debian/` before building:

```sh
cp -r packaging/debian .
dpkg-buildpackage -us -uc -b
```

Build-Depends: `debhelper-compat (= 13)`, `cargo`, `rustc` (1.95 or newer),
`pkg-config`, `libmpv-dev`.

Runtime dependencies: `libmpv2`, `ffmpeg`, plus what `${shlibs:Depends}`
finds (`libgl1`, `libegl1`, `libwayland-client0`, `libxkbcommon0`, `libc6`,
`libgcc-s1`). Recommends `libavcodec-extra` (the codecs Debian ships
separately) and the portals; suggests `mesa-va-drivers`,
`intel-media-va-driver` or `va-driver-all` for hardware decoding.

## Flatpak (untested)

Before the first build:

1. Pin the mpv commit in the manifest.
2. Generate the crate snapshot:
   `flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json`

```sh
flatpak-builder --user --install --force-clean build-dir \
  packaging/flatpak/io.github.crabplayer.CrabPlayer.yml
```

Flatpak requires the exported desktop file and icon to be named after the
app-id (`io.github.crabplayer.CrabPlayer`), while the binary announces the
app id `crab-player`. Check on the first build that the taskbar still shows
the icon; if not, the app id has to become configurable at build time.
