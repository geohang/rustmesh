# Contributing

Thanks for helping. Bug reports and pull requests are welcome; please keep a pull
request to one change.

## Build and test

You need a stable Rust toolchain (1.83 or newer for the library, 1.95 for the app)
and, for the Python package, Python 3.10 or newer.

```bash
cargo test -p rustmesh --features serde   # the mesher
cargo test -p rustmesh-app --lib agent    # sign-in and process tests
cargo xtask web                           # builds the web app into dist/web

pip install maturin numpy pytest
maturin develop                           # builds the extension into the active environment
pytest tests
```

The pyGIMLi tests are skipped when pyGIMLi is not installed. Run cargo from inside
the repository: it reads `.cargo/config.toml` from the current folder upwards. If
you build in a synchronised folder, set `CARGO_TARGET_DIR` to a folder outside it.
Windows build notes are in [docs/app.md](docs/app.md#building).

## Changes to the mesher

Add a test with a fix when a small one can show it. The tests are few on purpose:
they check results (areas, angles, markers, coverage of the input, determinism),
not implementation details. A change to meshing behaviour should say what it does
to the angle and area limits on a case that shows it.

Keep geometry validity, requested quality and solver accuracy apart in reports and
documentation: a valid mesh may miss its angle target, and a mesh that meets it
says nothing about the accuracy of a simulation on it.

Changes to `vendor/spade` are listed in `vendor/spade/PATCHES.md`; add a line there
and keep the modified-file notice at the top of the changed file.

## Style

Run `cargo fmt` on files you change, and keep comments short: say what the code
does or which rule it keeps. Documentation is plain English.

## Credentials

Never commit credentials, authorization URLs, callback codes, tokens or another
application's login cache. The sign-in tests generate a throwaway key at run time;
do not add key files.

## Licence

By contributing you agree that your contribution is licensed under the same terms as
the project, MIT OR Apache-2.0, without any additional terms.
