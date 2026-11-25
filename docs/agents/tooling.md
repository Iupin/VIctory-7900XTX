# Tooling: build, lint, test, asset policy

## Purpose

How the workspace is built, linted and tested (exactly as CI does), which
toolchain and lints apply, and how the static half of the asset policy
(`cargo xtask policy`) and its git hook work. The runtime half is
`victory_core::ensure_outside_repo` ([victory-core.md](victory-core.md)); the
rule is [docs/asset-policy.md](../asset-policy.md).

## How it works

### Toolchain

`rust-toolchain.toml` pins channel `1.98.1`, target `x86_64-pc-windows-msvc`,
components `rustfmt` and `clippy`, profile `minimal`. rustup picks it up in the
repository; CI runs `rustup toolchain install` to install exactly that. The
root `Cargo.toml` declares `rust-version = "1.88"` and `edition = "2024"` for
every crate (`[workspace.package]`); the pinned toolchain is what is actually
used. The MSVC build tools are needed (README).

### Workspace layout and profiles (`Cargo.toml`)

- `members = ["crates/*", "apps/*", "xtask"]`, `resolver = "3"`.
  `tests/fixtures` is excluded: fixture crates are standalone and built on
  demand by integration tests (the loader's).
- All shared dependency versions are in `[workspace.dependencies]`; crates use
  `.workspace = true`.
- `[profile.dev.package."*"] opt-level = 2`: dependencies are optimised even in
  debug builds (hashing and parsing of gigabytes of data).
- `[profile.release] debug = "line-tables-only"`.
- `[profile.diag] inherits = "release"`: same code generation, output in
  `target\diag`. Used with the `diagnostics` feature
  ([app-victory.md](app-victory.md)).

### Lints

Every crate has `[lints] workspace = true`. Workspace lints:

- Rust: `unsafe_op_in_unsafe_fn = "deny"`, `unused_must_use = "deny"`.
- Clippy: `all = warn` (priority -1, so the entries below override it). Allowed:
  `type_complexity`, `needless_range_loop`, `chunks_exact_to_as_chunks`,
  `sliced_string_as_bytes`, `collapsible_match`, `manual_clamp`,
  `implicit_saturating_sub`, `min_max`. The comment gives the reason for the
  first two (tuple-heavy signatures and indexed loops in the translator and
  GPU code, let through until that code is tidied); it says "two style lints"
  although eight are listed, and gives no reason for the other six.
- `undocumented_unsafe_blocks = "allow"`: upstream warns on unsafe blocks
  without a `// SAFETY:` comment; the comment says eleven blocks in this fork's
  newer GPU code still lack one. (That lint is in clippy's restriction group,
  not in `all`.)

Individual items still carry local `#[allow(...)]` where needed, e.g.
`clippy::too_many_arguments` on `texture::Layout::walk`.

CI passes `-D warnings`, so every warning of `clippy::all` fails the build.

### Building, linting and testing as CI does

`.github/workflows/ci.yml`: on pull requests and pushes to `main`, one job on
`windows-latest` (60 min timeout, read-only permissions, older runs of the same
ref cancelled). Steps:

```powershell
rustup toolchain install                       # the pinned toolchain
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

`Swatinem/rust-cache@v2` caches the target folder. There is no `cargo fmt`
check and no separate build step (clippy and test build everything). The tests
run without `--all-features`, so code behind `diagnostics` is linted but its
tests are not run (for example `apps/victory/src/replay.rs` `ranges`).
`--locked` means `Cargo.lock` must already match; adding a dependency needs the
lock file committed.

Locally, the same commands work. Build the binary used for play and
measurements with
`cargo build --locked --profile diag -p victory --features diagnostics`.

### `cargo xtask`

`.cargo/config.toml` defines the alias `xtask = "run --quiet --package xtask --"`.
`xtask/src/main.rs` has three commands:

- `cargo xtask policy [--staged]` -> `policy::run(root, staged)`.
- `cargo xtask docs` -> `docs::run(root)`: the code maps in `docs/agents/`
  agree with the code. It fails, one `DOCS:` line per problem, when a `.rs`
  file under `crates/`, `apps/`, `xtask/` or `tests/` is not named (its exact
  repo-relative path in backticks) in any `docs/agents/*.md`; when a map names
  such a path that no longer exists; when a `VICTORY_...` token in a `.rs`,
  `.ps1` or `.toml` file is missing from `docs/agents/switches.md`; or when
  `switches.md` names one the sources no longer have. Tokens are matched
  whole. It reads the working tree (there is no `--staged` mode). The test
  `repository_docs_are_current` runs it under `cargo test`.
- `cargo xtask install-hooks` runs `git config core.hooksPath .githooks` in the
  workspace root and fails if git fails. After that, `.githooks/pre-commit`
  (`cargo xtask policy --staged`, then `cargo xtask docs`) runs on every
  commit. Nothing is copied into `.git/hooks`.

The workspace root is `CARGO_MANIFEST_DIR/..` (`workspace_root`), so the
command checks this tree wherever it is run from.

### `cargo xtask policy` rules (`xtask/src/policy.rs`)

An allowlist. Which files:

- `--staged`: `git diff --cached --name-only --diff-filter=ACMR -z` (added,
  copied, modified, renamed in the index); content is read from the index with
  `git show :<path>`, so what is checked is what will be committed, not the
  working copy. A path that `git show` cannot read is skipped.
- Otherwise: `git ls-files -z --cached --others --exclude-standard`, i.e.
  tracked files **and untracked files that `.gitignore` does not exclude**,
  read from disk (unreadable or deleted ones skipped).
- Outside a git repository: every file under the root except `target` and
  `.git` directories.

Paths are compared with forward slashes. A path in `EXCEPTIONS` skips every
check. For all others, `check(path, content)` collects problems:

1. **File type.** The basename must be in `ALLOWED_NAMES` (`.gitignore`,
   `.gitattributes`, `.editorconfig`, `.victory-root`, `pre-commit`, `LICENSE`,
   `LICENSE-MIT`, `LICENSE-APACHE`) or its extension (after the last `.`,
   case-insensitive) in `ALLOWED_EXTENSIONS`: `rs toml md txt lock yml yaml
   json hlsl hlsli ps1 cmd sh`. Anything else, including files without an
   extension, fails.
2. **Size.** More than `MAX_FILE_SIZE` = 640 KiB (655,360 bytes) fails. The
   comment notes the translator's largest source file is 570 KiB.
3. **Known binary signatures.** Content that *starts with* one of
   `FORBIDDEN_MAGIC` fails whatever its extension: PE (`MZ`), shader container,
   DDS, RIFF, LLVM bitcode, PDB, `!<arch>` library, zip, 7-zip, cabinet,
   MSI/OLE.
4. **Binary content.** Any NUL byte fails.
5. **Encoded runs** (only if there was no NUL): the longest run of
   `[A-Za-z0-9+/=]` (`longest_encoded_run`) above `MAX_ENCODED_RUN` = 512
   characters fails as hex/base64.
6. **Hex literal runs** (only if 5 passed): `longest_hex_literal_run` adds up
   the hex digits of consecutive `\xNN` escapes or `0x...` literals separated
   only by commas, whitespace or backslashes (type suffixes such as `u32` and
   digit separators `_` are skipped); more than 512 digits fails. This catches
   byte strings and `xxd -i` style arrays that rule 5 sees as many short runs.
   Decimal arrays are not caught (a noted shortcut).

Violations print as `POLICY: <path>: <problem>` on stderr and the command
fails; success prints `asset policy: <n> files ok`.

**Exceptions** are entered in the `EXCEPTIONS` constant as
`(path, reason)` pairs; the path must match exactly, repository-relative with
forward slashes. The comment says to keep it empty if at all possible. The only
entry is `crates/victory-formats/src/bcpack-tables.bin` (the BCPack tables,
owner's decision of 2026-11-23). An exception also needs the file un-ignored
in `.gitignore` if its type is ignored there (that file has a `!` line for
it).

The xtask tests `accepts_source`, `rejects_game_like_files`,
`rejects_data_smuggled_into_text`,
`rejects_data_smuggled_as_an_escaped_byte_string` and
`rejects_data_smuggled_as_a_hex_byte_array` test `check` on synthetic input.
`repository_passes_policy` runs the full non-staged check on the real tree, so
`cargo test --workspace` (and CI) fails on any violation even in a clone
without the hook.

### Tests that need real files

The asset policy says such tests take paths from environment variables. In
the crates this map covers they are `#[ignore]` and **fail** (panic on
`unwrap`/`expect`) when run with `--ignored` and the variable unset; they do
not skip silently:

| Test | Variables | File |
|---|---|---|
| `reference_chunks` | `VICTORY_BCPACK_TABLES`, `VICTORY_BCPACK_CASES` | `crates/victory-formats/src/bcpack.rs` |
| `walks_saved_streams` | `VICTORY_XMA_STREAMS` | `crates/victory-formats/src/xma.rs` |
| `matches_a_reference_decoder` | `VICTORY_XMA_REFERENCE` | `crates/victory-formats/src/xma.rs` |

Run one with, for example,
`cargo test -p victory-formats walks_saved_streams -- --ignored --nocapture`.
`bcpack_throughput` and `imdct_throughput` are ignored benchmarks on synthetic
data. Elsewhere in the workspace (not covered here): `victory-gpu` has many
ignored tests that want particular `VICTORY_*` switches and a process of their
own, or a card, and some `victory-shader` tests return early when no D3D12 host
can be made (`Host::new` failing); the shader tests use `dxil.dll`, which
`VICTORY_DXIL` can point at (read in
`crates/victory-shader/src/emit/validator.rs`; see
[docs/README-fxceless.md](../README-fxceless.md)).

`apps/victory/src/run.rs` `cpu_feature_check_matches_host` reads
`VICTORY_SKIP_CPU_CHECK` but needs no files.

## Files

| Path | What it does | Key items |
|---|---|---|
| `xtask/src/main.rs` | `cargo xtask` CLI: `policy`, `docs`, `install-hooks`. | `Cli`, `Cmd`, `main`, `workspace_root` |
| `xtask/src/policy.rs` | The static asset-policy check and its tests. | `run`, `list_files`, `check`, `longest_encoded_run`, `longest_hex_literal_run`, `ALLOWED_EXTENSIONS`, `ALLOWED_NAMES`, `EXCEPTIONS`, `FORBIDDEN_MAGIC`, `MAX_FILE_SIZE`, `MAX_ENCODED_RUN`, `repository_passes_policy` |
| `xtask/src/docs.rs` | The code-map check and its tests. | `run`, `check`, `unmapped_sources`, `stale_paths`, `undocumented_switches`, `stale_switches`, `switch_tokens`, `backticked`, `repository_docs_are_current` |
| `.cargo/config.toml` | The `xtask` alias. | `[alias] xtask` |
| `.githooks/pre-commit` | Runs `cargo xtask policy --staged`, then `cargo xtask docs`. | - |
| `.github/workflows/ci.yml` | CI: two clippy runs and the tests on Windows. | job `check` |
| `rust-toolchain.toml` | Pinned toolchain 1.98.1. | `[toolchain]` |
| `Cargo.toml` | Members, shared dependencies, lints, profiles. | `[workspace.lints]`, `[profile.diag]` |
| `victory.example.toml` | Template for `victory.toml` (parsed by a victory-core test). | `[paths]` |
| `scripts/play.ps1` | Starts the title with this fork's settings; see [app-victory.md](app-victory.md). | `-Safe`, `-RayTracing`, `-Build`, `-More` |

## Switches

None read by the tooling itself. The ignored-test variables are listed above.

## Traps

- Untracked, non-ignored files count. A scratch file with a disallowed type or
  a long base64 line anywhere in the working tree makes `cargo test
  --workspace` fail through `repository_passes_policy`.
- Any allowed text file that happens to begin with `MZ`, `RIFF`, `DDS ` and so
  on is rejected; the magic check ignores the extension.
- Long unbroken identifiers, URLs without punctuation, or base64 in docs count
  as encoded runs (over 512 characters).
- The `"too big"` case in `rejects_game_like_files` writes 600 KiB of `a`,
  which is under the 640 KiB limit; it fails `check` because of the encoded-run
  rule, not the size rule. Raising or lowering the size limit is not covered by
  that test.
- `--staged` checks the index, not the working copy: a file fixed on disk but
  not re-added still fails the hook.
- `.gitattributes` forces LF for everything and CRLF for `*.ps1` and `*.cmd`.
- CI uses `--locked`; edit `Cargo.lock` together with any `Cargo.toml`
  dependency change.
- A new crate must add `[lints] workspace = true` to get the workspace lints.

## Where to start for common changes

- Allow a new text file type: `ALLOWED_EXTENSIONS` or `ALLOWED_NAMES` in
  `xtask/src/policy.rs`.
- Add an exception (only with the owner's decision): an entry in `EXCEPTIONS`
  with its reason, a `!` line in `.gitignore` if needed, and a note in
  `docs/asset-policy.md`.
- Change what CI runs: `.github/workflows/ci.yml`.
- Silence a clippy lint workspace-wide: `[workspace.lints.clippy]` in the root
  `Cargo.toml`, with a comment saying why.

## Origins

`xtask` and the policy check by Marvis (`f4504f6`, upstream VIctory). The
`\x`/`0x` hex-literal rule by LF2000 Builder (`20d9d1e`). This fork's
`3b30d6f` raised the size limit from 512 KiB to 640 KiB and added
`scripts/play.ps1`.
