# Contributing

Thanks for helping with Crab Player. Bug reports, fixes and packaging
improvements are welcome.

## Reporting a bug

Use the bug report template. The most useful things to include are your
distribution, desktop or compositor, GPU, how you installed Crab Player
(AppImage, .deb, .rpm, Arch, Flatpak, tarball or source) and the log:

```sh
RUST_LOG=crab_player=debug crab-player /path/to/file 2>&1 | tee crab-player.log
```

Security problems go through [SECURITY.md](SECURITY.md), not public issues.

## Changing the code

- Rust 1.95 or newer and the libmpv development files (`libmpv-dev`,
  `mpv-devel`, or `mpv` on Arch). `cargo build --no-default-features` builds
  the interface without libmpv.
- Before opening a pull request:
  ```sh
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo clippy --all-targets --no-default-features -- -D warnings
  cargo test && cargo test --no-default-features
  ```
  CI runs the same checks.
- `scripts/e2e-render-check.sh` and `scripts/middle-click-check.sh` check
  rendering and input under Xvfb; run them when you touch the video or
  input code.
- Every user action goes through a `Command` (`src/command.rs`), so menus,
  buttons, keys and mouse gestures behave the same; keep it that way.
- Keep pull requests focused and describe what you tested.

## Clean-room rule

Crab Player contains no code, artwork, icons or other assets taken from
other media players. Do not contribute anything copied or extracted from
third-party software; the only brand asset is the Crab Player logo in
`assets/`.

By contributing you agree that your work is released under the
[MIT License](LICENSE).
