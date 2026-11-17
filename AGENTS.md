# AGENTS.md

## What this is

BitChord Desktop Revived (`bitchord`) is a native desktop music client in
Rust — `egui`/`eframe` 0.37 for the UI, `rodio`/`symphonia` for audio, an
opt-in `wgpu` renderer, embedded `wry` WebViews for sign-ins. It plays from
YouTube Music and the fork's own sources (JioSaavn, MonoServers, addons),
Dolby Atmos from hi-res addons, and has Listen Together / jam sync through
its own relay server (`server/`).

It is a **daughter fork of [ytm-rs](https://github.com/grapedjuice/ytm-rs)**
and a sister fork of [BitChord-PC-Continued](https://github.com/jackychan4891/BitChord-PC-Continued) the style of the fork has to match Bitchord-PC-Continued rather than ytm-rs

most code comes from upstream, and new upstream commits get ported in
regularly. The fork's own features sit on top and must never be lost.

**Commit as:** `LoserDenji <loserdenji@proton.me>`

---

## Build & test

### System dependencies (Linux)

CI (ubuntu-24.04) installs:
```
libgtk-3-dev libwebkit2gtk-4.1-dev libasound2-dev \
libayatana-appindicator3-dev libdbus-1-dev libssl-dev pkg-config
```

Without sudo, the same works by downloading and extracting the debs to a
local prefix:

```bash
mkdir -p ~/gtk-local && cd /tmp/gtk-deps
apt-get download libgtk-3-dev libwebkit2gtk-4.1-dev libasound2-dev \
  libwebkit2gtk-4.1-0 libjavascriptcoregtk-4.1-0 libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-0 libsoup-3.0-dev libepoxy-dev libatk1.0-dev \
  libatk-bridge2.0-dev libpango1.0-dev libcairo2-dev libgdk-pixbuf-2.0-dev \
  libharfbuzz-dev libglib2.0-dev libdbus-1-dev libssl-dev \
  libfreetype-dev libfontconfig1-dev libffi-dev libpcre2-dev \
  libwayland-dev libxkbcommon-dev libx11-dev libxext-dev libxi-dev \
  libxrandr-dev libxcursor-dev libxfixes-dev libxcomposite-dev \
  libxdamage-dev libxinerama-dev libxtst-dev libxrender-dev libxcb1-dev \
  libxcb-render0-dev libxcb-shm0-dev libxau-dev libxdmcp-dev \
  libpixman-1-dev libpng-dev zlib1g-dev
for d in *.deb; do dpkg-deb -x "$d" ~/gtk-local/; done
```

Then:
```bash
export PKG_CONFIG_PATH=~/gtk-local/usr/lib/x86_64-linux-gnu/pkgconfig:~/gtk-local/usr/share/pkgconfig
export PKG_CONFIG_SYSROOT_DIR=~/gtk-local
export RUSTFLAGS="-L ~/gtk-local/usr/lib/x86_64-linux-gnu -L /usr/lib/x86_64-linux-gnu \
  -C link-arg=-Wl,-rpath,~/gtk-local/usr/lib/x86_64-linux-gnu"
export LD_LIBRARY_PATH=~/gtk-local/usr/lib/x86_64-linux-gnu:/usr/lib/x86_64-linux-gnu
```

Two finishing steps the build needs:
- Stub `.pc` files for `egl`, `gl`, `cloudproviders`, `atspi-2`, and
  `libsystemd` (version `250`) in the pkgconfig dir — several GTK `.pc`
  files require them but nothing links them here.
- `ln -sf libfoo.so.0 libfoo.so` for each library in
  `~/gtk-local/usr/lib/x86_64-linux-gnu/`, so the linker can find `-lgtk-3`
  etc.

### Commands

```bash
cargo check              # type-check — fast, needs no .so files
cargo check --tests      # also type-check the tests
cargo test --no-run      # compile tests — needs the .so files above
cargo test               # compile and run everything
cargo test module_name::     # one module's tests
```

`cargo check --tests` is the quick loop for fixing compile errors: it
skips linking, so it works even without the runtime libraries.

### CI

`.github/workflows/check.yml` runs `cargo test` and `cargo build --release`
on Linux, Windows and macOS. Warnings never fail the build — only errors
do. Windows and macOS need no system packages, so a change that compiles
locally on Linux is nearly always fine there too; what differs is
`#[cfg(windows)]`/`#[cfg(target_os = "macos")]` code, which the compiler
still type-checks for the local target only.

---

## Porting from upstream ytm-rs

1. Read the upstream diff: `git -C <ytm-rs> show <commit> -- src/`.
2. Never copy a whole file over the fork's version — the fork's files carry
   BitChord features upstream doesn't have. Apply the change surgically.
3. A cherry-pick's conflicts resolved to the upstream side is the classic
   way features get silently lost; when in doubt, keep the fork's hunk and
   re-apply upstream's change on top of it.
4. Upstream commits assume every *earlier* upstream commit is already in.
   When a ported change references something the fork doesn't have
   (`crate::backdrop`, `find::ListFind`, a struct field…), port the
   supporting piece too, never leave a stub.
5. Prefer commit messages like `fix/feat/port: <upstream message>`
   for hand-ported work; a bare cherry-pick with the upstream message is a
   sign the conflicts weren't resolved by hand.

---

## Code style

**Match the surrounding code.** Follow the naming, structure and comment
style of the files you are touching. Don't reformat code you aren't
changing — no drive-by rewraps, no import sorting, no `cargo fmt` over
whole files. The diff should read as only the change that was asked for.

**Don't trade features for speed.** Performance work keeps every existing
animation and visual. If a change alters anything visible, A frame is only
"spared" when nothing the listener sees is lost.

**Comments say why, not what.** A comment earns its line by explaining
something the code can't: the reason for a wait, a guard, a reorder.
Read like this:

```rust
// wait until the cover is in, otherwise the open spring jumps
let cover = app.cover_loaded(&song);

// paused while the player covers the page, so the bars stop eating frames
let playing = is_playing && !obscured;
```

Not like this:

```rust
/// NOTE: This function is responsible for retrieving the `cover` resource
/// and subsequently performing the initialization routine.
```

The second kind restates the code and rots the moment the code moves;
the first kind is the reason the line exists.
