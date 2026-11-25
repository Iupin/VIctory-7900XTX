# apps/victory (the `victory` binary)

## Purpose

The command-line front end. Each subcommand is a thin driver over the
libraries: it loads the config (`victory_core::Config`), walks the user's files
and calls into `victory-pe`, `victory-loader`, `victory-hle` (which re-exports
the `victory-gpu` pieces it needs), `victory-shader` and `victory-game`. It
writes only to stdout/stderr and to the user's cache and data folders. The
command list in upstream's words is in
[docs/README-fxceless.md](../README-fxceless.md); replay usage is in
[docs/replay.md](../replay.md).

## How it works

`main` (`apps/victory/src/main.rs`) calls `victory_core::logging::init("info")`
first. In a Windows diagnostics build it then asks
`victory_hle::modules::audio::capture::child()`: if this process was started
as the audio recorder child for `VICTORY_AUDIO_CAPTURE`, it does only that and
returns. Then clap parses `Cli { command: Command }` and dispatches.

`default_paths(paths)` is shared by several commands: the paths given, or the
configured `game_dir` plus `test_titles`; an error if both are empty.

Shader-finding commands share `apps/victory/src/shader.rs` `sources(data)`:
every SGD5 record (`victory_game::sgd5::records`) plus every raw container
found by its magic that `victory_shader::Container::parse` accepts. Files above
`MAX_FILE` (2 GiB) are skipped; files are read whole.

### Subcommands

**`inspect <file> [--imports] [--exports]`** (`inspect::run`). Parses one PE
with `victory_pe::ImageInfo::parse` and prints size, BLAKE3, CodeView PDB and
symbol key, kind, subsystem, image base and size, entry RVA, stack, DLL flags
(`dll_flags`), relocations/exception table/load config, TLS callbacks,
sections, imports per DLL (all symbols with `--imports`), delay imports and
exports (listed with `--exports`).

**`census [paths...] [--strict] [--missing]`** (`census::run`). Walks the paths
(`default_paths`), keeps files that start with `MZ` (`looks_like_pe`, any
extension), deduplicates by canonical path, parses each, and for every import
and delay import asks `victory_hle::HleResolver::classify` whether it is ours
(`Provenance::Hle`), the host's (`Host`), or missing. `--strict` uses
`HostPolicy::none()` so host pass-through does not count. Counts each
`dll!function` once per DLL across images (`FunctionKey`: name, hint ignored,
or ordinal). Prints a per-DLL table and totals; `--missing` lists missing
symbols by the number of images using them. A walk error at depth 0 (a path
that does not exist) fails the command; deeper errors are listed as skipped.
Title DLLs are never mapped here, so `Provenance::Title` is `unreachable!`.

**`lib-exports <lib> [--list]`** (`inspect::lib_exports`). Parses an import
library with `victory_pe::implib::ImportLibrary::parse` and prints exports per
DLL (each name with `--list`).

**`shader-info [paths...] [--verbose] [--disasm]`** (`shader::run`). For every
container found: DXIL or ACCC part, `DxilProgram::parse` (kind, shader model,
bitcode size, console `xdat`/`xbop` extras, standard layout),
`native_programs` (stages, code sizes) and `gfx10::decode` (instruction count,
opcodes not in the table, decode errors). `--verbose` prints one line per
container with the 16-digit BLAKE3 prefix a run's log uses; `--disasm`
(implies verbose) adds stage I/O, layout, used resources and the decoded
instructions (`format_instruction`).

**`translate [paths...] [--validate] [--pipelines] [--list] [--jobs N]
[--warm <file>]`** (`translate::run`, `translate::warm`). Before dispatch,
`main` applies `VICTORY_VIEW_LIMIT` (`emit::set_view_heap_limit`) and
`VICTORY_HOST_WAVE` (`emit::set_host_max_wave`).

- Without `--warm`: the offline survey. Refuses a `VICTORY_EMIT_DUMP` inside
  the repository, applies `VICTORY_HOST_MAX_WAVE` and `VICTORY_HOST_RDNA`,
  loads `Validator` (`--validate`; `VICTORY_DXIL` or the newest SDK's
  `dxil.dll`) and checks a `Host` can be made (`--pipelines`). For each file,
  `jobs` scoped threads (default: all logical cores) pull containers from
  `sources` by an atomic index and run `translate` -> `translate_programs`:
  `victory_shader::spill::build`, `structure::structure`, `resource::track`,
  `io::raise`, `emit::emit`, then `check` (validator, host pipeline with stub
  partner shaders, `--list` lines with a hash of the emitted module). Each
  thread keeps a `Tally` merged at the end; duplicates are skipped by the hash
  of the DXIL part alone. The report lists counts per stage and every failure
  reason with an example (`path@offset name #hash ~code8 Stage`).
  `VICTORY_PAIRS` builds the named vertex+pixel (or mesh+pixel) pipelines
  after the walk.
- With `--warm <file>` (or `--warm all`): collects 16-hex-digit hashes from the
  file, opens the shader cache (`d3d12x::use_shader_cache(cache/shaders)`) and
  translates only matching containers into it via
  `victory_hle::modules::d3d12x::warm_shader`, on `jobs` threads (default 4).
  `VICTORY_WARM_SECONDS` stops it after n seconds; it continues where it
  stopped next time.

**`shaders [paths...] [--jobs N]`** (`translate::precompile`). Opens the
shader cache and puts the translation of every distinct container found into
it with `victory_hle::modules::d3d12x::precompile`, which opens the host GPU
first so host-dependent settings match a run. Prints progress every 1000 and
failure reasons at the end. The README's "every shader translated, minutes".

**`warm [--jobs N] [--limit N] [--again] [--used]`** (Windows only;
`apps/victory/src/warm.rs` `run`). Makes the host pipelines of every pipeline
recorded by runs with `VICTORY_PIPELINE_BOOK=1` (`victory_hle::known_pipelines`,
which is `victory_gpu::pipeline::known`), so the driver's on-disk cache has
them before a run needs them. Folder: `<cache>/pipelines` (sibling of
`<cache>/shaders`). Reads `warm-done.txt` (keys done under the current
`known::fingerprint()`), `warm-bad.txt` and `early-pipelines.txt`; `--again`
redoes done keys, `--used` limits to the early (used) ones, which go first.
Work is cut into chunks of `AT_ONCE` (40); `jobs` threads (default two thirds
of the cores) each start a child `victory warm --these <n,...>` at idle
priority (`child`) and read its `made <n> <ms> <0|1>` lines. A child silent for
`LONGEST` (180 s) is killed; the pipeline it was at goes to `warm-bad.txt` and
the rest of the chunk goes to a new child. Pipelines taking at least 1 s are
appended to `slow-pipelines.txt`, which runs read back. The child side is
`make`: it calls `known::make` and ends with `std::process::exit(0)` to skip
normal shutdown. `--these` is hidden.

**`run <exe> [--execute] [--unimplemented trap|zero] [--strict]
[--console series-x|series-s] [-- <title args>]`** (`apps/victory/src/run.rs`
`run`). `main` first passes the trailing arguments to
`victory_hle::runtime::set_title_args`. Then, in order:

1. With `--execute`, `check_cpu_features` (AVX2, bypass with
   `VICTORY_SKIP_CPU_CHECK=1`).
2. `victory_hle::modules::lifecycle::wait_for_previous` (a relaunched title's
   old process must be gone).
3. `Config::load`, read the exe, `victory_loader::prepare`, print the build via
   `victory_game::identify` on the original BLAKE3.
4. `cache_subdir("images")`, `d3d12x::use_shader_cache(cache_subdir("shaders"))`
   (which also sets the pipeline cache next to it), `victory_loader::load`.
   The prepared and original bytes are dropped right after mapping to save
   memory.
5. Main image registered (`set_main_image`, `set_image_path`). Resolver from
   `HostPolicy::default()` or `none()` (`--strict`). The title folder's
   `MicrosoftGame.config` gives `runtime::Title` (id, storage quota; missing
   file: id 0). Storage is `<data>/<ID>/storage`, temp `<data>/<ID>/temp`;
   drives `G:` (package folder) and `T:` (temp) are mounted with
   `files::mount_at`. Console model set with `victory_hle::console::set`.
6. `victory_loader::load_title_dlls` maps the title's own DLLs (those that
   neither the host policy nor a registered HLE module serves: `title_dll`).
   `absent_title_dlls` names statically imported title DLLs not found; with
   `--execute` and `trap` that is a hard error (`--unimplemented zero` runs
   anyway).
7. `bind_imports` on every title DLL then the exe, through
   `victory_loader::WithTitleDlls`, printing a `BindReport` each.
8. Without `--execute`, stop here. With it: `write_sites::apply_asked`,
   `report_crashes`, `log_crashes_to(<data>/<ID>/crashes.log)`,
   `use_console_timers`, the diagnostics aids (below), the late-DLL loader,
   then `Arc::new(image).spawn_main(dlls)` and join; prints the exit code.

`LateDlls::load` is installed with `set_title_dll_loader` and serves the
title's own `LoadLibrary` of title DLLs nothing imports. Under `LoaderLock`
(re-entrant for the holding thread, exclusive otherwise, like the Windows
loader lock) it maps the DLL and its title dependencies, binds them against
HLE, earlier title DLLs and themselves, makes them visible, then runs each
`DllMain` (`attach`) on the calling thread. If one fails it detaches what this
load attached, forgets and unmaps all of it, and returns the error.

**`replay <folder> [...]`** (Windows diagnostics builds only;
`apps/victory/src/replay.rs`). Runs a captured frame again through
`victory_hle::replay::play` (= `victory_gpu::replay::play`). The clap struct
`Replay` has many flags; the modes, checked in this order, are: `--dump`,
`--listing`, `--flow`, `--retranslate`, `--slots`, `--images`, `--pixel`,
`--check-written`, `--upcopies`, `--watch`/`--diverge`/`--diverge-picture`
(with `--draws`, `--commands`/`--step`, `--lists`, `--roots`), else a timed
replay (`play::replay`, with `--compare` printing both runs and the median
difference). Run options common to all (`Options`): `--loops`, `--no-restore`,
`--timing lists|detailed`, `--top`, `--image`, `--buffers
as-recorded|vram|sysmem`, `--only-buffers`, `--no-barriers`,
`--merge-barriers`, `--skip-clamps`, `--batch-clamps`, `--skip-hash-passes`,
`--one-submit`, `--no-merge`, `--wait-all`, `--wait-lists`, `--pace`,
`--awake`, `--breadcrumbs`, `--only`, `--until`, `--skip-shader`,
`--shaders`, `--shaders-b`, `--debug-layer`. What each does is in the doc
comments of `Replay` and in [docs/replay.md](../replay.md).
`retranslate` collects the capture's containers into `containers` beside the
capture (searching `default_paths` for missing ones), sets host wave 64, RDNA,
`--view-limit` and `--raw-views`, and writes `<hash>.dxil` files into the
given folder on all cores.

**`config`** (`show_config`). Prints the config file used and every resolved
path (marking ones that do not exist) and runs `ensure_outside_repo` on each,
printing `POLICY VIOLATION: ...` rather than failing.

### The `diagnostics` feature and the `diag` profile

`apps/victory/Cargo.toml` feature `diagnostics` enables `diagnostics` in
`victory-core` (debug/trace logs; see [victory-core.md](victory-core.md)),
`victory-hle` (which enables it in `victory-gpu` and `victory-loader`),
`victory-loader` and `victory-shader`. In this crate it adds the `replay`
subcommand, the audio-capture child check in `main`, and `run.rs`
`diagnostics`: `victory_loader::dump_stacks_on_schedule`,
`victory_loader::profile_on_schedule(victory_hle::frame_phase)`,
`audio::capture::start`, `watch` (`VICTORY_WATCH`), `poke` (`VICTORY_POKE`)
and `victory_loader::save_on_crash` (`VICTORY_CRASH_SAVE`). The other crates
gate their own switches behind it (frame capture, the D3D12 debug layer,
`VICTORY_SKIP_PIPELINES`, and so on).

The `diag` profile (root `Cargo.toml`) is `inherits = "release"`: release code
generation (with `debug = "line-tables-only"` from `[profile.release]`), in
`target\diag` so it does not evict `target\release`. The build used for every
measurement is
`cargo build --locked --profile diag -p victory --features diagnostics`.
The feature and the profile are independent: `diag` without the feature is a
plain release build in another folder.

### `scripts/play.ps1`

Starts the title the way this fork is played. Parameters: `-Exe` (required),
`-Safe`, `-RayTracing`, `-Build diag|release` (default `diag`), `-More
@{NAME='value'}` (extra or overriding settings, applied last).

It removes every `VICTORY_*` variable from the session except
`VICTORY_CONFIG`, `VICTORY_GAME_DIR`, `VICTORY_CACHE_DIR` and
`VICTORY_DATA_DIR`, then sets:

| Always | Value | Read in |
|---|---|---|
| `VICTORY_TEXTURE_NO_WAIT` | `1` | `crates/victory-gpu/src/texture.rs` |
| `VICTORY_RELAY` | `2` | `crates/victory-gpu/src/host.rs`, `shadow.rs` |
| `VICTORY_WIDE_BACK` | `1` | `crates/victory-gpu/src/shadow.rs` |
| `VICTORY_CARD_REHASH_RUNS` | `1` | `crates/victory-gpu/src/host.rs` |
| `VICTORY_JOIN_COPIES` | `1` | `crates/victory-gpu/src/host.rs` |
| `VICTORY_INDIRECT_WINDOW` | `1` | `crates/victory-gpu/src/draw.rs` |
| `VICTORY_RELAY_COPY` | `1` | `crates/victory-gpu/src/host.rs` |
| `VICTORY_CARD_STEPS` | `2` | `crates/victory-gpu/src/host/card_steps.rs` |
| `VICTORY_INDIRECT_BATCH` | `2` | `crates/victory-gpu/src/draw.rs` |
| `VICTORY_MAP_THREADS` | `1` | `crates/victory-hle/src/modules/xmem.rs` |
| `VICTORY_TITLE_PRIORITIES` | `0` | `crates/victory-hle/src/modules/kernel.rs` |
| `VICTORY_CARD_AWAKE` | `1` | `crates/victory-gpu/src/host/awake.rs` |
| `VICTORY_SHOW_AFTER` | `1` | `crates/victory-gpu/src/descriptor/host.rs` |
| `VICTORY_AMBIENT` | `0.8` | `crates/victory-gpu/src/ambient.rs` |
| `VICTORY_D3D12_DRED` | `light` | `crates/victory-gpu/src/host.rs` |
| `VICTORY_SKIP_PIPELINES` | `none` | `crates/victory-gpu/src/pipeline.rs` (diagnostics only; `none` is not a 16-digit hash, so it is dropped with a warning and nothing is skipped) |

| Mode | Adds |
|---|---|
| default (not `-Safe`) | `VICTORY_THIN=1` (`list/thin.rs`), `VICTORY_JOIN_PARTS=8` (`host.rs`), `VICTORY_UPLOAD_AHEAD=1` (`popwatch.rs`, `texture/ahead.rs`) |
| `-Safe` | `VICTORY_UNDER_WIDE=0`, `VICTORY_LAND_ECHO=0` (both `shadow.rs`); the three above are left unset |
| `-RayTracing` | `VICTORY_BVH_BUILDER=1` (`pipeline.rs` `builder_runs`: the title's BVH builder pipelines are allowed to run) |

What each switch does is documented where it is read (see the GPU and HLE
maps). The script then checks that `target\<Build>\victory.exe` and the exe
exist, `Push-Location`s to the repository root (so `victory.toml` is found),
and runs `victory run <Exe> --execute`, popping the location afterwards. Its
header says to run it from a normal, not an elevated, window so the driver
keeps its shader cache in the user's place.

## Files

| Path | What it does | Key items |
|---|---|---|
| `apps/victory/src/main.rs` | CLI definition, dispatch, `config`, shared default paths. | `Cli`, `Command`, `Unimplemented`, `Console`, `main`, `show_config`, `default_paths` |
| `apps/victory/src/inspect.rs` | `inspect` and `lib-exports`. | `run`, `lib_exports`, `dll_flags` |
| `apps/victory/src/census.rs` | `census`: import coverage across images. | `run`, `tally_image`, `looks_like_pe`, `DllStats`, `FunctionKey` |
| `apps/victory/src/shader.rs` | `shader-info`, and the container finder the other shader commands share. | `run`, `sources`, `Source`, `MAX_FILE`, `format_instruction` |
| `apps/victory/src/translate.rs` | `translate` survey, `translate --warm`, `shaders` precompile. | `run`, `warm`, `precompile`, `translate`, `translate_programs`, `check`, `census_line`, `Tally`, `Checks` |
| `apps/victory/src/warm.rs` | `warm`: host pipelines made ahead in child processes. | `run`, `child`, `make`, `folder`, `AT_ONCE`, `LONGEST`, `SLOW_MS` |
| `apps/victory/src/run.rs` | `run`: prepare, map, bind, start; late title DLLs; diagnostics aids; CPU check. | `run`, `LateDlls`, `LateDlls::load`, `LoaderLock`, `title_dll`, `absent_title_dlls`, `check_cpu_features`, `diagnostics`, `watch`, `poke` |
| `apps/victory/src/replay.rs` | `replay` (Windows + diagnostics): options to `play::Options`, mode dispatch, `--retranslate`. | `Replay`, `run`, `retranslate`, `range`, `pair` |
| `apps/victory/tests/census.rs` | Runs the built binary: `census` fails on a missing path and reports `0 images scanned, 0 unreadable` on a file that is not a PE. | `census_fails_when_a_requested_path_is_missing`, `census_accepts_a_valid_path_with_no_pe_images` |

## Switches

Read in this crate (others are passed through to the libraries by `run`).

| Name | Default | Effect | Read in |
|---|---|---|---|
| `VICTORY_SKIP_CPU_CHECK` | unset | `1` skips the AVX2 check before `--execute` | `apps/victory/src/run.rs` |
| `VICTORY_CRASH_SAVE` | unset | diagnostics: `<rva>:<len>,...` (hex) saved to `<cache>/crash-<rva>.bin` on a crash | `apps/victory/src/run.rs` |
| `VICTORY_WATCH` | unset | diagnostics: `<rva>,...` (hex), logs the u32 at title+rva when it changes, polled every 5 ms | `apps/victory/src/run.rs` |
| `VICTORY_POKE` | unset | diagnostics: `<rva>:<byte>,...` (hex), writes the byte twice a second | `apps/victory/src/run.rs` |
| `VICTORY_VIEW_LIMIT` | unset | `translate`: view heap null slot to translate for | `apps/victory/src/main.rs` |
| `VICTORY_HOST_WAVE` | unset | `translate`: host max wave size | `apps/victory/src/main.rs` |
| `VICTORY_HOST_MAX_WAVE` | unset | `translate` (not `--warm`): host max wave size, set again | `apps/victory/src/translate.rs` |
| `VICTORY_HOST_RDNA` | unset | `translate` (not `--warm`): `1` translates as for RDNA | `apps/victory/src/translate.rs` |
| `VICTORY_PAIRS` | unset | `translate`: `vs+ps,...` label substrings to build as pipelines | `apps/victory/src/translate.rs` |
| `VICTORY_WARM_SECONDS` | unset | `translate --warm`: time limit | `apps/victory/src/translate.rs` |
| `VICTORY_TRANSLATE_ONLY` | unset | `translate`: only containers whose hash starts with one of these | `apps/victory/src/translate.rs` |
| `VICTORY_EMIT_DUMP` | unset | `translate`: directory for emitted DXIL (`--list`) and validator failures; refused inside the repo | `apps/victory/src/translate.rs` |
| `VICTORY_DUMP_MATCH` | unset | `translate`: `<dir>;<text>|<text>`, writes matching containers as `.cdxil` | `apps/victory/src/translate.rs` |
| `VICTORY_FAIL_DUMP` | unset | `translate`: first 3 containers of each failure reason as `.cdxil` | `apps/victory/src/translate.rs` |
| `VICTORY_SHADER_CENSUS` | unset | `translate`: file appended with per-program instruction counts | `apps/victory/src/translate.rs` |

`VICTORY_DXIL`, `VICTORY_AUDIO_CAPTURE`, `VICTORY_RECORD` and
`VICTORY_PIPELINE_BOOK` are named in doc comments here but read in other
crates.

## Traps

- Asset policy: only `VICTORY_EMIT_DUMP` is checked with
  `ensure_outside_repo` (test `emit_dump_inside_repo_is_refused`).
  `VICTORY_DUMP_MATCH`, `VICTORY_FAIL_DUMP`, `VICTORY_SHADER_CENSUS` and
  `replay --retranslate` write title-derived data (containers, translations,
  shader labels) to the place given without that check. Point them outside the repository.
- `VICTORY_HOST_WAVE` (main.rs) and `VICTORY_HOST_MAX_WAVE` (translate.rs) both
  call `set_host_max_wave`; the second wins in `translate` without `--warm`.
  `--warm` sees only the first.
- `translate::warm` and `precompile` share `use_shader_cache` with `run`, so
  they fill the same cache a run reads; `precompile` opens the host first so
  the cache key matches the run's host (see the GPU map, `shader_cache`).
- `run` prints with `println!` (stdout) and logs to stderr. Tools that parse
  its output read stdout.
- `run` drops the file bytes after `load` on purpose (memory). Code that needs
  the original later must keep its own copy.
- `LateDlls::load` adds new DLLs to `mapped` before their `DllMain`s run, so a
  `DllMain` that loads another DLL can bind against them. Changing that order
  breaks re-entrant loads.
- `warm` children exit with `process::exit(0)`: nothing after `known::make`
  runs (no saving, no host thread shutdown). A child's stdout must stay the
  `made`/`at` protocol; any other print there is ignored.
- `replay` and its `ranges` test exist only with `--features diagnostics` on
  Windows; CI's `cargo test` runs without features, so that test is not run
  there (clippy with `--all-features` still checks it).
- `apps/victory/tests/census.rs` needs the binary built
  (`CARGO_BIN_EXE_victory`), which `cargo test` does.
- `run.rs` comments name the title and a version, and give one code offset.
  Keep new comments to our own names (asset policy, and fxceless' `4e17478`
  removed the title's name from the tree).

## Where to start for common changes

- New subcommand: a variant in `Command` and a match arm in `main`
  (`apps/victory/src/main.rs`), code in a new module; get paths with
  `default_paths`, write only through `Config::cache_subdir`/`data_subdir`.
- What `run` does before the title starts: `apps/victory/src/run.rs` `run`,
  steps in the order above.
- Title `LoadLibrary` of its own DLLs: `LateDlls::load`.
- New counters in the shader survey: `Tally` and `translate_programs` in
  `apps/victory/src/translate.rs`.
- Pipeline warm-up files or child protocol: `apps/victory/src/warm.rs` and
  `victory_gpu::pipeline::known`.
- New default setting for playing: `scripts/play.ps1` `$settings`.

## Origins

Initial `main.rs`, `inspect.rs`, `census.rs`, `run.rs`, `shader.rs` by Marvis
(`f4504f6`, `754619f`, upstream VIctory); `translate.rs` from Marvis'
`754619f`. Later: on-disk size in `inspect` by Thomas Askharoun (`e8eb93a`);
census deduplication by DoubleD17 (`6d24aaa`) and nathanbzrr (`67d1d02`),
traversal failures and `apps/victory/tests/census.rs` by Bloxive (`cd6c15f`); AVX2
pre-flight check by David Palacios (`467bb36`); missing title DLLs message by
AgneyaK (`9c63a43`); `VICTORY_POKE` by Marvis (`3f0055e`); stage I/O in
`shader-info` by FIocker (`d02932f`); emitted-shader listing by Aws
(`c754111`); `VICTORY_EMIT_DUMP` repo check by LF2000 Builder (`886afce`);
per-lane descriptors by r4. (`6638098`) and "Claude" (`077ec8b`); lane-mask
pairs by cxccxc (`90228a7`); fork changes by fxceless (`578d459`, `1684989`,
`8dd30ab`). `replay.rs`, `warm.rs` and `scripts/play.ps1` are this fork's
(`d7ecfd2`, `b7ab9c4`, `a30829c`, `3b30d6f`).
