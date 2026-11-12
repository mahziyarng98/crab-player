# Flathub submission

The Flatpak is ready to submit, but it is **not on Flathub yet** and the pull
request has not been opened. This file records the exact steps and the files
that go into the submission.

## Policy note

Flathub's requirements and pull request template do not allow an AI agent to
open or automate a submission pull request, or to write the submission
commit message, description or review replies, and they do not allow
AI-generated or AI-assisted manifest content. The manifest and metadata here
were prepared as ordinary project files; the submission itself must be
opened, described and maintained by a person. That is why this directory
contains instructions instead of a link to an open PR.

## What goes into the pull request

From `packaging/flatpak/`:

- `io.github.mahziyarng98.crab-player.yml` — the manifest. Its crab-player
  module is a git source at the `v0.1.1` tag and that tag's commit; update
  both (and `cargo-sources.json`) for later releases.
- `cargo-sources.json` — every crate in that tag's `Cargo.lock`, generated
  with
  [`flatpak-cargo-generator.py`](https://github.com/flathub/flatpak-builder-tools/tree/master/cargo).
  The build is offline, so this file is required.

Nothing else is needed: there are no patches and no `flathub.json`.

## Steps

1. Fork `flathub/flathub` and clone the fork.
2. Branch off `new-pr` (the branch is intentionally empty):
   `git switch -c add-io.github.mahziyarng98.crab-player origin/new-pr`.
3. Create the directory `io.github.mahziyarng98.crab-player/` and copy the
   two files above into it.
4. Check the files with the same linter Flathub runs:
   ```sh
   flatpak run --command=flatpak-builder-lint org.flatpak.Builder \
       manifest io.github.mahziyarng98.crab-player/io.github.mahziyarng98.crab-player.yml
   ```
   The AppStream metadata lives in the app repository, not in this
   directory. Check it there before submitting:
   ```sh
   flatpak run --command=flatpak-builder-lint org.flatpak.Builder \
       appstream packaging/io.github.mahziyarng98.crab-player.metainfo.xml
   ```
5. Commit, push, and open a pull request against `new-pr` titled
   `Add io.github.mahziyarng98.crab-player`, using the repository's template.
   It asks for a short screen recording of the app playing a video; record
   one on the test machine. Answer the AI-use questions truthfully.
6. Wait for the Flathub CI build and a reviewer. Do not ping reviewers or
   post the link elsewhere.

## Why each permission is requested

| Permission | Reason |
| --- | --- |
| `--share=ipc` | X11 fallback; required by the toolkit for shared memory |
| `--socket=wayland` | Native Wayland window |
| `--socket=fallback-x11` | Runs on X11 only where Wayland is unavailable |
| `--device=dri` | GPU rendering and VA-API hardware decoding |
| `--socket=pulseaudio` | Audio output |
| `--share=network` | File ▸ Open URL and network streams (`http`, `rtsp`, …) |
| `--filesystem=xdg-videos:ro` | Drag and drop from a file manager passes plain paths, not portal documents; read-only access covers the common case |

Files opened through the file chooser and "Open with" use the document
portal. Save Image uses the save dialog's portal, so no pictures permission
is needed.

## Version updates

After a new release tag exists, regenerate `cargo-sources.json` from that
tag's `Cargo.lock`, update the manifest's `tag` and `commit`, and open a pull
request in the Flathub repository with only those two files changed.
