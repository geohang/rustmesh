# Third-party notices

rustmesh is MIT OR Apache-2.0. Builds of it also contain the code below, under
its own licences.

## vendor/spade

A modified copy of spade 2.15.1 by Stefan Altmayer (Copyright (c) 2017 The
Spade Developers; MIT OR Apache-2.0), packaged as `rustmesh-spade`. Its licence
files are in `vendor/spade/`, and `vendor/spade/PATCHES.md` lists the changes.

## Rust crates

The library, the Python module and Mesh Studio are compiled together with Rust
crates from crates.io under MIT, Apache-2.0, Zlib, BSD-2-Clause, BSD-3-Clause,
ISC, BSL-1.0 or Unicode-3.0 terms, or a choice among them; a crate that also
offers GPL-2.0 (self_cell) is used under Apache-2.0. `cargo tree -p <package>
-e normal` lists the crates of a build.

For the web app, `cargo xtask web` (or `cargo xtask licenses` alone) writes the
licence text of every crate in it to `dist/web/THIRD_PARTY_LICENSES.txt`. The
website publishes that file with the app:
<https://geohang.github.io/rustmesh/app/THIRD_PARTY_LICENSES.txt>.

## Fonts in Mesh Studio

Mesh Studio, on the desktop and on the web, draws its text with egui's default
fonts from the `epaint_default_fonts` crate:

| font | licence |
|---|---|
| Hack Regular | MIT (Copyright 2018 Source Foundry Authors); its Bitstream Vera glyphs are under the Bitstream Vera License |
| Noto Emoji Regular | SIL Open Font License 1.1 |
| Ubuntu Light | Ubuntu Font Licence 1.0 |
| emoji-icon-font | MIT (Copyright (c) 2014 John Slegers) |

Their licence texts are in `THIRD_PARTY_LICENSES.txt` under
`epaint_default_fonts`.
