# Publishing to the AUR (manual steps — not automated)

Crab Player's Arch packaging lives in this repo under `packaging/arch/`:

- `PKGBUILD` — the **release** package (`crab-player`), building from the
  GitHub tag archive `v<pkgver>.tar.gz` with a pinned `sha256sums`.
- `PKGBUILD-git` — the **development** package (`crab-player-git`), building
  from the `main` branch (`git+https://…`).

Both were validated in a clean `archlinux:base-devel` container
(`makepkg` as a non-root builder) and run through `namcap` with no errors.
The `namcap` warnings about `mpv`, `ffmpeg` and the GL/windowing libraries
are expected: the player opens those at runtime with `dlopen`/`dlsym`, so
`namcap` cannot see them, but they are real runtime requirements.

The AUR packages are **not created or updated by any automated process in
this repo**. You do it yourself, by hand. The steps below are exactly what
to run.

## 0. What you need once

1. Create an AUR account at <https://aur.archlinux.org> (same username is
   fine).
2. Add an SSH key to your account:
   <https://aur.archlinux.org/account/sshkeys>. The AUR uses SSH
   (port 22, `aur.archlinux.org`, user `aur`); the key in `~/.ssh/id_ed25519.pub`
   works if you already use SSH for GitHub.
3. Install `base-devel` (for `makepkg`) and `namcap` on any Arch system:
   ```sh
   sudo pacman -S --needed base-devel namcap
   ```

## 1. Release package: `crab-player`

```sh
# Fresh clone of the AUR package (creates ./crab-player/)
git clone ssh://aur@aur.archlinux.org/crab-player.git
cd crab-player

# Copy the current PKGBUILD and generated .SRCINFO from this repo.
cp /path/to/crab-player/packaging/arch/PKGBUILD .
cp /path/to/crab-player/packaging/aur/.SRCINFO .

# Sanity: verify the checksum matches the current release and build locally.
makepkg -f
namcap PKGBUILD crab-player-*.pkg.tar.zst

# If pkgver/source changed for a new release, regenerate .SRCINFO:
#   makepkg --printsrcinfo > .SRCINFO
# and update sha256sums with:  updpkgsums

git add PKGBUILD .SRCINFO
git commit -m "Update to 0.x.y"
git push
```

## 2. Development package: `crab-player-git`

```sh
git clone ssh://aur@aur.archlinux.org/crab-player-git.git
cd crab-player-git

cp /path/to/crab-player/packaging/arch/PKGBUILD-git PKGBUILD
cp /path/to/crab-player/packaging/aur/.SRCINFO-git .SRCINFO

makepkg -f
namcap PKGBUILD crab-player-git-*.pkg.tar.zst
# Regenerate .SRCINFO before every push (pkgver tracks the current commit):
#   makepkg --printsrcinfo > .SRCINFO

git add PKGBUILD .SRCINFO
git commit -m "Sync with upstream"
git push
```

## 3. After every release

- `crab-player`: bump `pkgver`, run `updpkgsums` (or set `sha256sums`
  to the new tag archive's sha256), regenerate `.SRCINFO`, push.
- `crab-player-git`: nothing to do — it rebuilds from `main`; just update
  `.SRCINFO` when pushing.

## 4. Rules to respect

- Never push to the AUR from a script or CI. The AUR is a human-maintained
  space; automated pushes are unwelcome and risky.
- Keep `pkgrel=1` unless the same `pkgver` changes (then bump `pkgrel`).
- The AUR does not allow binary uploads — only `PKGBUILD`, `.SRCINFO`,
  and small helper files.
