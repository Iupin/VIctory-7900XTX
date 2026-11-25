# victory-core

## Purpose

Shared plumbing that every other crate and binary uses: where the user's files
live (`Config`), the runtime half of the asset policy (`ensure_outside_repo`),
and the one `tracing` setup. It depends on nothing of ours. `apps/victory` loads
the config and initialises logging; `victory-pipeline::Cache::open` and every
writer of derived data call the policy guard. See
[docs/asset-policy.md](../asset-policy.md) for the rule itself.

## How it works

### Config loading and precedence

`Config::load` (`crates/victory-core/src/config.rs`):

1. `explicit_config_path` reads `VICTORY_CONFIG`; an empty value counts as unset.
2. The file is `VICTORY_CONFIG` if set, else `victory.toml` **relative to the
   current directory** (not the repository root, not the executable's folder).
   If `VICTORY_CONFIG` names a missing file, `load` fails. A missing
   `victory.toml` is fine: every value falls back.
3. The file is parsed into `FileConfig { paths: FilePaths }`. Both structs are
   `#[serde(deny_unknown_fields)]`, so a misspelt key or an extra section is an
   error, not ignored.
4. `Config::resolve` applies, per value: environment variable, then file, then
   default.

| Field | Env | File key | Default |
|---|---|---|---|
| `game_dir: Option<PathBuf>` | `VICTORY_GAME_DIR` | `[paths].game_dir` | none |
| `cache_dir: PathBuf` | `VICTORY_CACHE_DIR` | `[paths].cache_dir` | `default_dir("cache")` |
| `data_dir: PathBuf` | `VICTORY_DATA_DIR` | `[paths].data_dir` | `default_dir("data")` |
| `test_titles: Vec<PathBuf>` | `VICTORY_TEST_TITLES` (split by `std::env::split_paths`, `;` on Windows) | `[paths].test_titles` | empty |
| `source: Option<PathBuf>` | - | - | the file read, if any |

Empty env values are ignored for every variable (`env_path` filters them, and
`VICTORY_TEST_TITLES` is checked for emptiness separately). The env overrides
the file value as a whole; nothing is merged inside a value.

`default_dir(name)` is `%LOCALAPPDATA%\victory\<name>`, else
`$XDG_CACHE_HOME/victory/<name>`, else `$HOME/.cache/victory/<name>`; if none
of those is set, `resolve` fails. Both `cache` and `data` use the same base, so
on Linux `data` ends up under the cache home too.

`load` does **not** check the asset policy and does not create directories.
That happens when a caller asks for a sub-directory:
`Config::cache_subdir(sub)` and `Config::data_subdir(sub)` both go through
`subdir`, which calls `ensure_outside_repo(<root>/<sub>)` and then
`create_dir_all`. `Config::require_game_dir` returns the game dir or an error
naming the two ways to set it. The `victory config` subcommand
(`apps/victory/src/main.rs` `show_config`) prints the resolved values and runs
`ensure_outside_repo` on every path, printing `POLICY VIOLATION` instead of
failing.

`victory.example.toml` at the root is the template; the test
`example_config_parses` includes it with `include_str!`, so renaming a key in
`FilePaths` without updating the example fails the tests.

### The asset-policy guard

`ensure_outside_repo(path)` (`crates/victory-core/src/policy.rs`) returns
`Ok(())` only if `path` is outside every VIctory source tree. The path need not
exist.

- `resolve` makes the path absolute (against the current directory),
  canonicalises the longest prefix that exists, re-appends the missing tail and
  `normalize`s it lexically (`.` dropped, `..` pops). A trailing `..` whose
  `file_name()` is `None` is kept in the tail on purpose so `normalize` sees it.
- Check 1: the source tree this binary was compiled from
  (`compiled_workspace_root`, from `CARGO_MANIFEST_DIR/../..`) is canonicalised;
  if it still holds the `ROOT_MARKER` file and the resolved path starts with
  it, the result is `PolicyError::InsideRepo`.
- Check 2: every ancestor of the resolved path is tested for a
  `.victory-root` file (`ROOT_MARKER`). This catches other clones and copies of
  the repository, wherever they are.
- `PolicyError::Io` comes back only if nothing of the path can be canonicalised
  (for example the current directory cannot be read).

`.victory-root` is a tracked text file at the repository root explaining
itself; `xtask/src/policy.rs` lists it in `ALLOWED_NAMES`. Deleting or moving
it disables both checks for this tree (check 1 also requires the marker).

The static half of the rule is `cargo xtask policy` (see
[tooling.md](tooling.md)).

### Logging

`logging::init(default_level)` (`crates/victory-core/src/logging.rs`) builds a
`tracing_subscriber` registry with three layers: a `LevelFilter` cap, an
`EnvFilter`, and a `fmt` layer with targets, writing to **stderr**. The filter
is `VICTORY_LOG`, else `RUST_LOG`, else `default_level` (`apps/victory` passes
`"info"`). A filter string that does not parse is silently replaced by the
default. `try_init` is used and its error dropped, so a second call is a no-op.

The cap `CAP` is `TRACE` with the crate feature `diagnostics`, else `INFO`. A
build without the feature therefore never prints `debug!`/`trace!`, whatever
the filter asks for. `apps/victory`'s `diagnostics` feature turns on
`victory-core/diagnostics`.

## Files

| Path | What it does | Key items |
|---|---|---|
| `crates/victory-core/src/lib.rs` | Module list and re-exports. | `Config`, `PolicyError`, `ensure_outside_repo` |
| `crates/victory-core/src/config.rs` | Resolves the user's paths from env, file and defaults; hands out policy-checked sub-directories. | `Config::load`, `Config::resolve`, `cache_subdir`, `data_subdir`, `require_game_dir`, `explicit_config_path`, `default_dir`, `FileConfig`, `FilePaths` |
| `crates/victory-core/src/policy.rs` | Refuses destinations inside a VIctory source tree. | `ensure_outside_repo`, `ROOT_MARKER`, `PolicyError`, `resolve`, `normalize`, `compiled_workspace_root` |
| `crates/victory-core/src/logging.rs` | One tracing subscriber for every binary, capped at INFO without `diagnostics`. | `init`, `CAP` |

## Switches

| Name | Default | Effect | Read in |
|---|---|---|---|
| `VICTORY_CONFIG` | unset: `./victory.toml` | config file to read; must exist if set | `crates/victory-core/src/config.rs` |
| `VICTORY_GAME_DIR` | file value | the user's dump | `crates/victory-core/src/config.rs` |
| `VICTORY_CACHE_DIR` | file, then `%LOCALAPPDATA%\victory\cache` | derived data root | `crates/victory-core/src/config.rs` |
| `VICTORY_DATA_DIR` | file, then `%LOCALAPPDATA%\victory\data` | what titles save; `crashes.log` | `crates/victory-core/src/config.rs` |
| `VICTORY_TEST_TITLES` | file value | other titles, `;`-separated | `crates/victory-core/src/config.rs` |
| `VICTORY_LOG` | `RUST_LOG`, then `info` | tracing filter, e.g. `victory_loader=trace,info` | `crates/victory-core/src/logging.rs` |

`LOCALAPPDATA`, `XDG_CACHE_HOME` and `HOME` are read by `default_dir`.

## Traps

- `victory.toml` is found relative to the current directory. `scripts/play.ps1`
  does `Push-Location` to the repository root for this reason. Running
  `target\diag\victory.exe` from another folder silently uses defaults.
- `deny_unknown_fields`: adding a key to `victory.toml` that `FilePaths` does
  not know breaks every command that loads the config.
- `Config::load` does not validate anything; only `cache_subdir`/`data_subdir`
  (and `Cache::open`) apply the guard. Code that joins onto `cfg.cache_dir`
  directly bypasses it (`apps/victory/src/run.rs` `diagnostics` does this for
  `crash-<rva>.bin`; it is under the cache dir, which `cache_subdir` already
  checked earlier in `run`).
- The guard compares canonical paths. A path that cannot be canonicalised at
  all yields `PolicyError::Io`, which callers treat as a refusal.
- `compiled_workspace_root` is baked in at compile time. A binary copied
  elsewhere still knows where it was built; check 2 covers the copy's own tree.
- Debug logs need both the feature and a filter. "My `debug!` prints nothing"
  usually means a build without `--features diagnostics`.
- The tests `cache_inside_repo_is_refused` and `data_inside_repo_is_refused`
  depend on `.victory-root` being present.

## Where to start for common changes

- New configurable path: add it to `Config`, `FilePaths` and `resolve` in
  `crates/victory-core/src/config.rs`, document it in `victory.example.toml`,
  print it in `show_config` in `apps/victory/src/main.rs`.
- New tool that writes derived data: get the directory from `cache_subdir` or
  `data_subdir`, or call `ensure_outside_repo` yourself before writing.
- Log format or destination: `init` in `crates/victory-core/src/logging.rs`.

## Origins

Initial `config.rs`, `policy.rs` and `logging.rs` by Marvis (`f4504f6`, upstream
VIctory). Later: empty `VICTORY_TEST_TITLES` and `VICTORY_CONFIG` ignored by
Enzo Souza Duarte (`cc63aa2`, `41dc5da`); `data_dir` shown and checked by
Advait Palve (`9a8908c`); `..` in the missing tail resolved by nino-exe
(`bb0a242`).
