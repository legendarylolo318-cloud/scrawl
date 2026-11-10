# Inkwell

Handwritten-math notes in pure Rust + [Slint](https://slint.dev). Draw with a mouse, trackpad, pen
tablet, or an iPad/Android device; Inkwell recognizes each symbol and typesets it. Typing is fully
supported and mixes freely with drawing. Linux and Windows. No Chromium, no webview.

> **Status: work in progress.** The core crates are implemented and tested, but the desktop UI does
> not yet expose everything (notebooks sidebar, rich-text editing, tablet panel and more are library-level
> only). [`REVIEW_NOTES.md`](REVIEW_NOTES.md) lists exactly what works, what is partial and what is missing.

## What works today

- Ink canvas: pen, highlighter, eraser, smoothing, scribble-to-delete, undo/redo, zoom/pan, paper backgrounds.
- Recognition: 191 classes (digits, Latin, Greek, operators, relations, calculus, sets/logic, arrows, blackboard bold...),
  90.5 % top-1 / 99.5 % top-5 on the HASYv2 test split, under 1 ms per symbol on CPU, 2.9 MB model.
- Candidate strip (keys 1-5), automatic personal training from corrections, "Train my handwriting" mode.
- Expression tree -> Typst / LaTeX / Unicode; spatial parsing of scripts, fractions, roots, sums, integrals.
- Library crates: typing shorthand + autocomplete + template editor, storage (Typst + JSON, atomic, trash,
  history, backups, encryption), search, export (PDF/PNG/MD/Typst/LaTeX/HTML), plotter, PDF import,
  Opus audio, tablet server.

## Install

Download from the GitHub release page: a Linux AppImage, a Windows portable zip, or a Windows installer.
Verify with `SHA256SUMS`. Nothing else to install; no GPU needed.

## Build

```bash
cargo build --release -p inkwell
./target/release/inkwell              # run
./target/release/inkwell --self-test  # headless smoke test (exit 0 = OK)
```

Linux build dependencies (Arch): `pacman -S alsa-lib fontconfig libxkbcommon wayland dbus pkgconf`.
Debian/Ubuntu: `libasound2-dev libfontconfig1-dev libxkbcommon-dev libwayland-dev libdbus-1-dev pkg-config`.

## Model training (optional; end users never need this)

Training is also pure Rust (`burn`) and runs on any GPU through Vulkan/DX12 (AMD, NVIDIA, Intel incl.
integrated) or on the CPU:

```bash
cargo run --release -p inkwell-train -- download
cargo run --release -p inkwell-train -- train --epochs 30
cp training/out/symbols.onnx training/out/labels.json crates/recog/model/
```

Details in [`training/README.md`](training/README.md). To adapt to *your* handwriting you do not need to
retrain: use the in-app **Train** button and draw each symbol a few times.

## Tablet as a drawing tablet

Off by default. Click "Connect tablet" (UI wiring in progress; library in `crates/remote`), install the
local CA once on the device, scan the QR code, compare the 6-digit code, approve. Full design, threat
model and reset instructions: [`SECURITY.md`](SECURITY.md).

## Data and attribution

- **HASYv2** (Martin Thoma, 2017), <https://zenodo.org/records/259444>, licensed under the Open
  Database License (ODbL) v1.0. Used for training the recognizer; attribution required, share-alike applies to
  the database itself. Commercial use is permitted.
- Synthetic training data is rendered from DejaVu and Libertinus fonts (free licenses). No non-commercial
  datasets are used.
- Typst (Apache-2.0), Slint (see their licensing terms), tract, burn, tantivy, rustls, rcgen, age, hayro,
  opus-pure and other crates under permissive licenses; see `Cargo.lock`.

## License

MIT. See [`LICENSE`](LICENSE).
