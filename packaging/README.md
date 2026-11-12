# Packaging

What each format is, how it is built and what was tested:
[`docs/packaging.md`](../docs/packaging.md).

| Path | Builds |
| --- | --- |
| `io.github.mahziyarng98.crab-player.desktop` | desktop entry shared by every format |
| `io.github.mahziyarng98.crab-player.metainfo.xml` | AppStream metadata shared by every format |
| `appimage/build.sh` | `dist/Crab_Player-<version>-x86_64.AppImage` and the tarball (Ubuntu 22.04 container) |
| `debian/build.sh` | `dist/crab-player_<version>-1_amd64.deb` (Debian 12 container) |
| `rpm/build.sh`, `rpm/crab-player.spec` | `dist/crab-player-<version>-1.fc45.x86_64.rpm` (Fedora container) |
| `arch/build-local.sh`, `arch/PKGBUILD`, `arch/PKGBUILD-git` | `target/arch/crab-player-<version>-1-x86_64.pkg.tar.zst` (host makepkg) |
| `flatpak/build.sh`, `flatpak/io.github.mahziyarng98.crab-player.yml`, `flatpak/cargo-sources.json` | `dist/Crab_Player-<version>-x86_64.flatpak` |
| `flathub/README.md` | how the Flathub submission is made (not submitted yet) |
| `tarball.sh` | `dist/crab-player-<version>-linux-x86_64.tar.gz` |
| `test-in-container.sh` | installs one of the above in a clean container and plays a clip |

The container builds use docker or podman; each script also has an
`--inside` mode for CI jobs that already run in the right image.
