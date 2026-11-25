# Switches: every `VICTORY_...` environment variable

The code reads about 340 `VICTORY_...` environment variables. Each one is
explained in a comment where it is read; this file lists them all in one
place. It covers every `VICTORY_[A-Z0-9_]+` token in the sources (`.rs`,
`.ps1`, `.toml`, `.yml`, `.md`, leaving out `target/` and `docs/agents/`).
Tokens that are not environment variables are listed under
[Not switches](#not-switches).

Each row was written from the code that reads the variable, not from the
comment alone. Where the two disagree, the code wins, and the row says so.
"(not verified)" marks what the code read for this file does not settle.

## How switches are read

**No central registry.** Most switches are read with `std::env::var` /
`var_os` right next to the code they change, and the result is cached in a
`static OnceLock` (or `LazyLock`) inside a small function. So a switch is
read **once, the first time that code runs**, not at start-up and not on
every call. Changing the environment of a running process has no effect.
Some switches are only reached late (the first draw, the first device loss),
so a mistyped value goes unnoticed until then. A few are read on every call
with no cache: `VICTORY_ONE_PAD` (`one_pad`), `VICTORY_INDIRECT_WIDE`
(`widest`), `VICTORY_INDIRECT_REST` (`rest`), and the ones read once while
the device or the process is set up (`VICTORY_D3D12_BACKGROUND`,
`VICTORY_EFFICIENCY_MODE`, `VICTORY_PROCESS_PRIORITY`).

**On/off patterns.** Read the code before trusting a "1": the code uses five
different patterns.

| Pattern in the code | Unset | `0` | `1` | Any other value | Used for |
|---|---|---|---|---|---|
| `is_none_or(\|v\| v != "0")` | on | off | on | on | the "on by default; `=0`: as before" switches, most of them |
| `is_some_and(\|v\| v == "1")` | off | off | on | **off** | the "off unless `=1`" experiments |
| `is_some_and(\|v\| v != "0")`, and `crate::switch::on` | off | off | on | on | e.g. `VICTORY_SEND_CHANGED`, `VICTORY_SHOW_AFTER`, `VICTORY_UPLOAD_AHEAD`, `VICTORY_D3D12_DEBUG` |
| `is_some()` / `is_none()` | off | **on** | on | on | e.g. `VICTORY_CAPTURE_WORLD`, `VICTORY_GPU_TIMING` in `late_stage`, `VICTORY_ROOT_SRV_DIRECT`, `VICTORY_WAVE_WHY`, `VICTORY_STRICT_JOINS` |
| `match` on text | the `_` arm | as listed | as listed | the `_` arm | multi-value switches (`VICTORY_LATE_WAIT`, `VICTORY_INDIRECT_BATCH`, ...) |

Numbers are parsed with `.trim().parse().ok()` (some without `trim`), and a
value that does not parse falls back to the default without a warning. The
bit sets of the shader translator (`VICTORY_SHADER_OPT`, `VICTORY_RDNA_FLOAT`,
`VICTORY_SHADER_LOOP_LIMIT`) also take `0x` hex. Path variables in
`victory-core` treat an empty value as unset.

**The helpers.**

- `crates/victory-gpu/src/switch.rs` `on(name)`: on when set and not exactly
  `0` (`value_is_on`; `""`, `"true"`, `"00"` and `" 0"` are on). It is not
  cached; its callers cache it.
- `crates/victory-gpu/src/switch.rs` `toggled!("NAME")`: on unless exactly
  `0`, read once into a `OnceLock`. In builds with the `diagnostics` feature,
  when `VICTORY_AB` names the switch, its value comes from `crate::ab` and
  changes while the title runs.
- `crates/victory-gpu/src/titlewrite.rs` `asked_for!("NAME")`: off unless
  exactly `1`. `VICTORY_AB` can drive it as well.
- `crates/victory-gpu/src/ab.rs` `pick(name)` / `bit(name)` and
  `victory_gpu::switched(name)` (`crates/victory-gpu/src/lib.rs`): the same
  A/B hook, used by hand where a switch has more than two values (for
  example `VICTORY_JOIN_PARTS`: A/B on means 8 and off means 1).
- `crates/victory-core/src/config.rs` `Config::load` / `resolve`: the path
  variables. The order is environment, then the config file
  (`VICTORY_CONFIG`, else `./victory.toml`), then the default. An empty
  value counts as unset. `VICTORY_CONFIG` naming a missing file is an error.

**Diagnostics-only switches** are marked **[diag]**. Their code is compiled
only with the `diagnostics` feature (whole modules such as `capture`,
`copystat`, `book`, `faultbook`, `popwatch`, `replay`, `ab`, `timeline::record`,
`host::panel`, `host::spans`, `host::upcheck`, `loader::profile`,
`loader::stacks`, `audio::capture`, or an explicit
`cfg!(feature = "diagnostics")`). In a plain release build such a variable is
ignored. `target\diag` is built with `--features diagnostics` (README.md).
**[A/B]** marks switches that `VICTORY_AB` can change while the title runs
(diagnostics builds only).

**The shader cache key.** A switch that changes what the translator emits
must be part of the key, or a run with the switch gets translations cached
without it. `crates/victory-gpu/src/shader_cache.rs` `build` keys on an
explicit list of switch values: `VICTORY_WAVE_WIDE`, `VICTORY_WAVE_DEEP_SKIPS`,
`VICTORY_WAVE_OWN_BITS`, `VICTORY_WAVE_LOCAL`, `VICTORY_F16_NEAREST`,
`VICTORY_HELPER_OWN`, `VICTORY_FIRSTLANE_OWN`, `VICTORY_FIRST_HALF`,
`VICTORY_STRICT_JOINS`, `VICTORY_MESH_OUTPUT_STRICT`, `VICTORY_BVH_FENCES` and
`VICTORY_BVH_BOUNDS`. It also keys on the effects of `VICTORY_SHADER_LOOP_LIMIT`,
`VICTORY_GROUP_SYNC` and `VICTORY_SHADER_OPT`, and on `emit::host_settings()`
(host wave, RDNA bits, view-heap limit). Those rows are marked **[key]**.
Separately, `crates/victory-shader/build.rs` collects every `"VICTORY_..."`
string literal in `crates/victory-shader/src` into
`VICTORY_SHADER_SWITCHES`. Only `emit::translation_fingerprint` uses that
list, and a grep finds no caller of that function, so in practice the
explicit list in `shader_cache.rs` is the key.

### How to add a switch

1. Read it next to the code it changes, in a small function with a
   `static OnceLock` (copy one of the `is_none_or(|v| v != "0")` functions).
   For a plain on/off switch in `victory-gpu`, use `crate::toggled!("<name>")`
   (default on) or `crate::switch::on("<name>")` cached in a `OnceLock`
   (default off). Then it accepts `0` the same way the others do, and
   `VICTORY_AB` can cycle it.
2. Above the function, write a doc comment in the house style: what the
   switch does, its default, what each value does, and what `=0` restores
   ("as before"). Name any run or measurement it rests on. README.md points
   readers to these comments.
3. If it only matters for diagnostics, put it behind
   `#[cfg(feature = "diagnostics")]` or `cfg!(feature = "diagnostics")`.
4. If it changes translated shader code, add it to the list in
   `crates/victory-gpu/src/shader_cache.rs` `build`, or the shader cache
   serves stale translations.
5. Keep any parsing in a pure function and test it (`switch::value_is_on`,
   `emit::parse_shader_opt`, `host::wide_refresh_enabled` are examples).
6. Add a row here. If `scripts/play.ps1` should set it, add it there too.
   The script clears every other `VICTORY_*` variable before it starts the
   title.

## `scripts/play.ps1`

The script removes every `VICTORY_*` variable from the window's environment
except `VICTORY_CONFIG`, `VICTORY_GAME_DIR`, `VICTORY_CACHE_DIR` and
`VICTORY_DATA_DIR`. It then sets the following and runs
`target\<Build>\victory.exe run <exe> --execute` from the repository root.

- **Always:** `VICTORY_TEXTURE_NO_WAIT=1`, `VICTORY_RELAY=2`,
  `VICTORY_WIDE_BACK=1`, `VICTORY_CARD_REHASH_RUNS=1`, `VICTORY_JOIN_COPIES=1`,
  `VICTORY_INDIRECT_WINDOW=1`, `VICTORY_RELAY_COPY=1`, `VICTORY_CARD_STEPS=2`,
  `VICTORY_INDIRECT_BATCH=2`, `VICTORY_MAP_THREADS=1`,
  `VICTORY_TITLE_PRIORITIES=0`, `VICTORY_CARD_AWAKE=1`, `VICTORY_SHOW_AFTER=1`,
  `VICTORY_AMBIENT=0.8`, `VICTORY_D3D12_DRED=light`,
  `VICTORY_SKIP_PIPELINES=none`.
- **Without `-Safe`:** `VICTORY_THIN=1`, `VICTORY_JOIN_PARTS=8`,
  `VICTORY_UPLOAD_AHEAD=1`.
- **With `-Safe`:** `VICTORY_UNDER_WIDE=0` and `VICTORY_LAND_ECHO=0`, and the
  three above are left unset.
- **With `-RayTracing`:** `VICTORY_BVH_BUILDER=1`.
- `-More @{NAME='value'}` adds or overrides entries.

Some of these values are already the defaults (`VICTORY_RELAY=2`,
`VICTORY_WIDE_BACK=1`). `VICTORY_SKIP_PIPELINES=none` is not 16 hex digits,
so `parse_skip_hashes` skips nothing; in a diagnostics build it also logs a
warning. Rows below mark the script's values as **[play: value]**.

## Configuration and paths

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_CONFIG` | `./victory.toml` if it exists | path; empty = unset | Which config file is read; an error if it names a missing file. | `crates/victory-core/src/config.rs` `explicit_config_path` |
| `VICTORY_GAME_DIR` | `[paths].game_dir` of the config | path; empty = unset | The user's dump of the title. | `crates/victory-core/src/config.rs` `Config::resolve` |
| `VICTORY_CACHE_DIR` | `[paths].cache_dir`, else `%LOCALAPPDATA%\victory\cache` | path | Root of derived data; `cache_subdir` refuses one inside the repository. | `crates/victory-core/src/config.rs` `Config::resolve` |
| `VICTORY_DATA_DIR` | `[paths].data_dir`, else `%LOCALAPPDATA%\victory\data` | path | What titles save (persistent storage), kept apart from the cache. | `crates/victory-core/src/config.rs` `Config::resolve` |
| `VICTORY_TEST_TITLES` | `[paths].test_titles` | path list (`std::env::split_paths`, `;` on Windows) | Other titles used to exercise platform code. | `crates/victory-core/src/config.rs` `Config::resolve` |
| `VICTORY_LOG` | `RUST_LOG`, else the binary's default level | `EnvFilter` directives, e.g. `victory_loader=trace,info` | Log filter; debug and trace exist only in **[diag]** builds. An unparsable value falls back to the default. | `crates/victory-core/src/logging.rs` `init` |

## Loader and HLE (CPU side, runtime modules)

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_SKIP_CPU_CHECK` | off | `1` only | Skips the AVX2 pre-flight check (e.g. under an emulator). | `apps/victory/src/run.rs` `skip_cpu_check` (name in `SKIP_CPU_CHECK_ENV`) |
| `VICTORY_EFFICIENCY_MODE` | the process refuses Windows' efficiency mode | `1` only | `1` leaves Windows free to throttle the process when its window is in the background. | `crates/victory-hle/src/runtime.rs` `use_console_timers` |
| `VICTORY_PROCESS_PRIORITY` | above normal | `normal` or `0` | Keeps the process at its starting priority. | `crates/victory-hle/src/runtime.rs` `ahead_of_other_programs` |
| `VICTORY_TITLE_PRIORITIES` | title's two lowest priorities become below normal | `0` only | `0`: thread priorities as the title sets them. **[play: 0]** | `crates/victory-hle/src/modules/kernel.rs` `SetThreadPriority` (with `title_priority`) |
| `VICTORY_WINDOW_MAX` | the work area | `WIDTHxHEIGHT` (invalid: warning, ignored) | A smaller limit for the title's window, without changing its render targets. | `crates/victory-hle/src/modules/lifecycle.rs` `window_limit` (used by `fit`) |
| `VICTORY_ONE_PAD` | on | `0` | One merged pad fed by keyboard and every host pad; `0`: each host pad separate. Read on every call. | `crates/victory-hle/src/modules/gameinput.rs` `one_pad` |
| `VICTORY_KEYS` **[diag]** | none | `<s>:<key>,...` or `<from>-<to>:<key>` | Scripted key presses (also the first pad's buttons). | `crates/victory-hle/src/modules/gameinput.rs` `scripted` |
| `VICTORY_DSTORAGE_IO` | `handles` (`Io::DEFAULT`) | `one`, `handles`, `overlapped`, `direct` | How the DirectStorage stand-in reads files (one handle, one per worker, overlapped, past the file cache). | `crates/victory-hle/src/modules/dstorage.rs` `Io::chosen` |
| `VICTORY_DSTORAGE_HALF` | off | `1` only | A queue submits itself once half of it is enqueued. | `crates/victory-hle/src/modules/dstorage.rs` `half_submits` |
| `VICTORY_DSTORAGE_RETRIES` | 12 | count | How many times a write into pages is retried before the request fails (about 1.5 s for 12). | `crates/victory-hle/src/modules/dstorage.rs` `into_pages` |
| `VICTORY_DSTORAGE_LOG` | off | file (any value turns it on) | Streaming log: a line per request and status entry. | `crates/victory-hle/src/modules/dstorage.rs` `log` |
| `VICTORY_DSTORAGE_TRACE` | 0 | 0, 1, 2 | Streaming trace: 1 batches and suspect requests, 2 every request. | `crates/victory-hle/src/modules/dstorage.rs` `trace` |
| `VICTORY_SECTION_WINDOW` | on | `0` | One view of the memory section is kept for DirectStorage writes; `0`: a view per write. | `crates/victory-hle/src/modules/xmem.rs` `window` |
| `VICTORY_STREAM_NOTICE` | on | `0` | Tells the GPU side where DirectStorage bytes arrived; `0`: not told (then `VICTORY_PAGE_WRITES` decides). | `crates/victory-hle/src/modules/xmem.rs` `stream_notice` |
| `VICTORY_MAP_LEAN` | on | `0` | Mapping changes skip placeholder splits and commits that are already done. | `crates/victory-hle/src/modules/xmem.rs` `lean` |
| `VICTORY_STREAM_PRIORITY` | on | `0` | Lifts the title's below-normal streaming thread above normal where it maps and submits. | `crates/victory-hle/src/modules/xmem.rs` `lift_thread` |
| `VICTORY_MAP_THREADS` | 8 | number 1-32, or `auto` | Threads that map a group of pages side by side. **[A/B]** (on = 8, off = 1). **[play: 1]** | `crates/victory-hle/src/modules/xmem.rs` `asked_threads`, `side_by_side` |
| `VICTORY_CLZERO_ASK` | off (answers remembered per thread) | `1` only | `1`: Windows is asked about memory at every emulated `clzero`, as before. | `crates/victory-loader/src/console_insn.rs` `remembers` |
| `VICTORY_PATCH_WAITS` | on | `0` | `monitorx`/`mwaitx` are patched in the title's code at the first fault; `0`: emulated every time. | `crates/victory-loader/src/console_insn.rs` `patches_waits` |
| `VICTORY_PATCH_CLZERO` | on | `0` | A `clzero` loop is replaced by plain stores at its first fault; `0`: emulated every time. | `crates/victory-loader/src/console_insn.rs` `patches_clzero` |
| `VICTORY_WRITE_SITES` | off | file | Store sites in the title's code are changed at load to look before they write (loader). The GPU side also writes places it learns into the same file. | `crates/victory-loader/src/write_sites.rs` `apply_asked`; `crates/victory-gpu/src/titlewrite.rs` `learn` |
| `VICTORY_IMAGE_WATCH` | off | `name=<hex offset>:<1\|2\|4\|8>,...` | Logs words of the title's image once a second. | `crates/victory-hle/src/image_watch.rs` `start` |
| `VICTORY_XMA_DUMP` **[diag]** | off | dir | Saves context words and input buffer 0 of the first 32 voices. | `crates/victory-hle/src/modules/audio.rs` `dump_input` |
| `VICTORY_AUDIO_CAPTURE` **[diag]** | off | `.wav` file | Records what the title plays (process loopback) through a child recorder process. | `crates/victory-hle/src/modules/audio/capture.rs` `start`, `child` |

## GPU submission and threading

The defaults of `VICTORY_RELAY` and `VICTORY_LATE_WAIT` depend on each
other. Read these two rows together.

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_LATE_WAIT` | 3 while the relay is on (the default), else 0 | `1`, `2`, `3`; anything else 0 | Submissions go to the card without a wait each; 2: title bytes put in exactly under flights' writes; 3: the present is a flight of its own. Forced to 0 when `VICTORY_GPU_TIMING` is set to anything. | `crates/victory-gpu/src/host.rs` `late_stage` |
| `VICTORY_RELAY` | on, stage 2 (not in the crate's tests) | `1`, `2`; any other value off | A thread of ours lands what shaders wrote, beside the submitting thread; `1`: the first stage, which waits for everything. Off with `VICTORY_KEEP_SNAPSHOTS=1`. The comments in `host.rs` still say "off by default"; `shadow::relay` turns it on when unset. **[play: 2]** | `crates/victory-gpu/src/shadow.rs` `relay`; `crates/victory-gpu/src/host.rs` `relay`, `relay_exact` |
| `VICTORY_RELAY_COPY` | off | `1` only | The relay's fetches use the card's copy queue, not the drawing queue. **[play: 1]** | `crates/victory-gpu/src/host.rs` `relay_copy` |
| `VICTORY_LATE_WAIT_LISTS` | 256 | count, at least 1 | Most host lists in the air before flights are settled. | `crates/victory-gpu/src/host.rs` `late_most` |
| `VICTORY_LATE_WAIT_WRITES` | on | `0` | A submission with bottom-of-pipe writes may fly; `0`: settled at once. | `crates/victory-gpu/src/host.rs` `late_writes` |
| `VICTORY_FILL_TITLE` | off | `1` only | A small upload fill or buffer copy of memory the title wrote since does not wait for flights. | `crates/victory-gpu/src/host.rs` `fill_is_titles` |
| `VICTORY_JOIN_PARTS` | 1 | count | Parts of submissions handed to the host as one (only when refreshes run ahead of writes). **[A/B]** (on = 8, off = 1). **[play: 8, not with -Safe]** | `crates/victory-gpu/src/host.rs` `joins_asked` |
| `VICTORY_JOIN_LISTS` | 48 | count | Most lists joined into one hand-over. | `crates/victory-gpu/src/host.rs` `joins_asked` |
| `VICTORY_JOIN_COPIES` | off | `1` only | A title buffer copy that continues the one before it in the list becomes one step. **[play: 1]** | `crates/victory-gpu/src/host.rs` `join_copies` |
| `VICTORY_ASYNC_SUBMIT` | off (on when `VICTORY_THIN=1`) | `1` only | Submissions are prepared and sent by a thread of ours; needs the relay with the present sent ahead, and cut waits. **[A/B]** | `crates/victory-gpu/src/queue/behind.rs` `on`; `crates/victory-gpu/src/queue.rs` `frame_ends` |
| `VICTORY_SUBMIT_DEPTH` | 64 | count, at least 1 | Submissions that may be in that line when a submit call returns. | `crates/victory-gpu/src/queue/behind.rs` `runs_most` |
| `VICTORY_SUBMIT_FRAMES` | 1 | 0 or 1 | Presents that may be in the line when a present returns. | `crates/victory-gpu/src/queue/behind.rs` `frames_most` |
| `VICTORY_THIN` | off | `1` only | List calls only write their arguments down; our threads translate them; turns on the submission line. **[play: 1, not with -Safe]** | `crates/victory-gpu/src/list/thin.rs` `wanted` |
| `VICTORY_THIN_THREADS` | 8 | 1-32 | Threads that translate thin lists. | `crates/victory-gpu/src/list/thin.rs` `start` |
| `VICTORY_QUEUE_WAIT` | on | `0` | Queues honour fence waits; `0`: every submission runs when made. | `crates/victory-gpu/src/queue.rs` `waits` |
| `VICTORY_SPLIT_WAITS` | on | `0` | Submissions are held list by list at unmet waits; `0`: whole submissions. | `crates/victory-gpu/src/queue.rs` `split` |
| `VICTORY_CUT_WAITS` | on (needs the two above) | `0` | Lists are cut into host lists at their waits for memory values. | `crates/victory-gpu/src/queue.rs` `cuts` |
| `VICTORY_WAIT_FORCE` | on | `0` | A present runs what queues still hold; `0` keeps it held (can stop the title). | `crates/victory-gpu/src/queue.rs` `forces` |
| `VICTORY_CUT_WRITES` | off | `1` only | The host list is cut right behind a bottom-of-pipe value write (needs `cuts`). | `crates/victory-gpu/src/list.rs` `value_written` |
| `VICTORY_WAIT_ASK` | off | `1` only | `VirtualQuery` before a wait value is read, as before. | `crates/victory-gpu/src/list.rs` `holds` (inside the wait's `met`) |
| `VICTORY_LATE_WAITS` | off | `1` only | A blocking frame-event wait sleeps to the next tick, as before. | `crates/victory-gpu/src/frame.rs` `wait` (with `next_tick`) |
| `VICTORY_CARD_STEPS` | 0 | `1`, `2` | Steps that read memory flights may write are done on the card in order: 1 constant fills, 2 buffer copies too; only with relay stage 2. **[play: 2]** | `crates/victory-gpu/src/host/card_steps.rs` `stage` |
| `VICTORY_PLACED_ORDER` | off | `switch::on` | A copy at its place in a list blocks card steps only if they stand behind it. **[A/B]** | `crates/victory-gpu/src/host/card_steps.rs` `placed_order` |
| `VICTORY_COPIES_IN_PLACE` | on | `0` | The title's buffer copies are done on the card at their place in the list; `0`: on the CPU at submission. | `crates/victory-gpu/src/host/place.rs` `on` |
| `VICTORY_SEND_OWN` | on | `0` | What goes up to card copies is what it was when the submission was prepared. | `crates/victory-gpu/src/host/send.rs` `own` |
| `VICTORY_UNDER_WORDS` | on (needs `VICTORY_SEND_OWN`) | `0` | Title bytes under flights' writes go in by whole words, not changed bytes. | `crates/victory-gpu/src/host/send.rs` `words_under` |
| `VICTORY_SEND_BAR` | off | `1` only | The send memory lies in card memory written by the CPU over the bus. | `crates/victory-gpu/src/host/send.rs` `on_card` |
| `VICTORY_FETCH_CHECK` | off | `1` only | Pieces fetched while a later flight wrote them are found and put right (with `VICTORY_RELAY_COPY`). | `crates/victory-gpu/src/host/torn.rs` `on` |
| `VICTORY_LAND` | on (needs `VICTORY_SAFE_COPY` and `VICTORY_EXACT_BACK` on) | `0` | Shader writes are merged into title memory on a few threads; `0`: piece by piece on the relay's thread. | `crates/victory-gpu/src/land.rs` `on` |
| `VICTORY_LAND_THREADS` | logical CPUs / 4, clamped 1-4 | 1-16 | Landing threads, the relay's own counted. | `crates/victory-gpu/src/land.rs` `threads` |
| `VICTORY_LAND_BESIDE` | on | `0` | Landings write through our own view of the physical pages, past the guards. | `crates/victory-gpu/src/land.rs` `beside_on` |
| `VICTORY_LAND_SKIP` | off | `1` only | Pieces an earlier fetch already holds as they are now are left out. | `crates/victory-gpu/src/land.rs` `skip_on` |
| `VICTORY_LAND_ANYTIME` | on | `0` | A piece goes through our view even while the runtime changes a mapping. | `crates/victory-gpu/src/land.rs` `anytime` |
| `VICTORY_LAND_TAKEN` | on | `0` | A copy a landing went through counts as up to date with it. | `crates/victory-gpu/src/land.rs` `taken` |
| `VICTORY_UNDER_THREADS` | 6 | 1-16 | Threads for title bytes under flights' writes. | `crates/victory-gpu/src/land.rs` `each_under` |
| `VICTORY_VIEW_THREADS` | 4 for views, 6 for heap records | 1-16 | Threads that make descriptors. | `crates/victory-gpu/src/land.rs` `each_view` |
| `VICTORY_WIDE_REFRESH` | on with 4+ logical CPUs | `0`, `1` (force) | A submission's copy refreshes run on several threads. | `crates/victory-gpu/src/host.rs` `wide_refresh` / `wide_refresh_enabled` |
| `VICTORY_REFRESH_AHEAD` | on (needs wide refresh and `VICTORY_LIGHT_REFRESH`) | `0` | Copies the shaders write are refreshed together ahead of the steps. | `crates/victory-gpu/src/host.rs` `ahead_of_writes` |
| `VICTORY_REFRESH_FRONT` | off (on when `VICTORY_JOIN_PARTS` > 1) | `switch::on` | Everything a submission refreshes is done in one go ahead of its steps. | `crates/victory-gpu/src/host.rs` `refresh_front` |
| `VICTORY_WIDE_FILLS` | on (needs wide refresh) | `0` | Upload-memory fills done together on several threads. | `crates/victory-gpu/src/host.rs` `wide_fills` |
| `VICTORY_WIDE_TEXTURES` | on (needs wide refresh) | `0` | A submission's textures are refreshed on several threads. | `crates/victory-gpu/src/host.rs` `wide_textures` |
| `VICTORY_WIDE_MERGE` | off | `1` only | Shader writes to card copies are merged into title memory on several threads. | `crates/victory-gpu/src/host.rs` `wide_merge` |
| `VICTORY_HEAP_PASS_ASIDE` | on | `0` | The pass over the heap's buffer views runs on a thread of ours while the title records. | `crates/victory-gpu/src/host/aside.rs` `on` |
| `VICTORY_FAST_CORES` | 2 | 0-3 | Which of our threads ask Windows for the fast cores (0 none ... 3 most). | `crates/victory-gpu/src/cores.rs` `stage` |
| `VICTORY_CARD_AWAKE` | off | `1`, `turns` (20 s on, 20 s off) | Keeps the card's clocks up between submissions. **[play: 1]** | `crates/victory-gpu/src/host/awake.rs` `mode` |
| `VICTORY_WAIT_RESIDENT` | on | `0` | New card memory is made resident, and waited for, before anything can name it. | `crates/victory-gpu/src/host.rs` `wait_resident` |
| `VICTORY_KEEP_FREED` | on | `0` | Memory given to the card stays with it while there is room. | `crates/victory-gpu/src/host.rs` `keep_freed` |
| `VICTORY_SWEEP` | on | `0` | The once-a-second sweep that lets go of what nothing names; `0` keeps everything. | `crates/victory-gpu/src/host.rs` `sweep` |
| `VICTORY_BURY_LAG` | 64 | submissions; `0` none | How much later released objects are dropped and finished host lists reset. | `crates/victory-gpu/src/host.rs` `bury_lag` |
| `VICTORY_LAZY_PRELUDE` | on | `0` | A host list's prelude is made only when the list first needs one. | `crates/victory-gpu/src/host.rs` `lazy_prelude` |
| `VICTORY_SCRATCH_POOL` | on | `0` | Lists share scratch chunks; `0`: 4 MiB chunks kept per list. | `crates/victory-gpu/src/host.rs` `scratch_pool` |
| `VICTORY_LIST_POOL` | on | `0` | Host lists that have run are reused. | `crates/victory-gpu/src/list.rs` `pools` |
| `VICTORY_INDIRECT_CARD_BUDGET` | on | `0` | A list's indirect-work budget lives in card memory, not CPU-mapped memory. | `crates/victory-gpu/src/host.rs` `card_budget` |
| `VICTORY_D3D12_BACKGROUND` | left as the runtime has it | `0` only | `0` turns off the driver's background shader recompilation (`SetBackgroundProcessingMode`). | `crates/victory-gpu/src/host.rs` `Host::create` |
| `VICTORY_DISPLAY_MODE` | 3840x2160 | `WxH` | The display size the console reports to the title. | `crates/victory-gpu/src/dxgi.rs` `display_size` |
| `VICTORY_GPU_LOAD_MS` | 0 (timestamps as they are) | ms | What the title's own GPU timestamps report as its GPU time. | `crates/victory-gpu/src/queue.rs` `gpu_load` |

## Title memory and write tracking

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_SAFE_COPY` | on | `0` | Copies to/from title memory are one `rep movsb` whose faults the `watch` handler takes (installs it); `0`: through the kernel. | `crates/victory-gpu/src/safecopy.rs` `on` |
| `VICTORY_COPY_ASK` | off | `1` only | `VirtualQuery` before copying into a title destination, as before. | `crates/victory-gpu/src/host.rs` `copy_mapped` |
| `VICTORY_DIRECT_WRITES` | on | `0` | Title memory is written with `NtWriteVirtualMemory`, not `WriteProcessMemory`. | `crates/victory-gpu/src/host.rs` `direct_writes` |
| `VICTORY_COMMITTED_QUERY` | off | `1` only | Committed-memory questions go to `VirtualQuery`, as before. | `crates/victory-gpu/src/texture.rs` `by_query` |
| `VICTORY_VIEW_GUARD` | on | `0` | All views of a guarded physical page are guarded together. | `crates/victory-gpu/src/watch.rs` `view_guard` |
| `VICTORY_PAGE_PARTS` | on | `0` | The 4 KiB parts the kernel names are kept and copied, not the whole 64 KiB page. | `crates/victory-gpu/src/watch.rs` `parts_on` |
| `VICTORY_TAKE_PARTS` | on (`toggled!`) | `0` | A refresh takes only the parts of a page written since. **[A/B]** | `crates/victory-gpu/src/watch.rs` `take` |
| `VICTORY_ASK_EARLY` | off | ms (unparsable: 2) | A thread asks about written memory between a submission's askings. | `crates/victory-gpu/src/watch.rs` `early` |
| `VICTORY_ASK_ONCE` | on | `0` | A submission asks the kernel once about every range it refreshes. (The comment above the function still says "off by default".) | `crates/victory-gpu/src/watch.rs` `once` |
| `VICTORY_LIGHT_REFRESH` | on | `0` | The light refresh path (bits of view memory, unchanged copies skipped early). | `crates/victory-gpu/src/watch.rs` `light` |
| `VICTORY_KNOWN_HOLES` | on (needs light refresh) | `0` | Placeholders are known, not looked at at every asking. | `crates/victory-gpu/src/watch.rs` `known_holes` |
| `VICTORY_OPEN_ONE_VIEW` | on (`toggled!`) | `0` | A write fault opens only the view written. **[A/B]** | `crates/victory-gpu/src/watch.rs` `open_one_view` |
| `VICTORY_GUARD_REST` | 3000 us (`toggled!`) | microseconds > 1; `0` off | How long a block the title just opened by a write stays open. **[A/B]** | `crates/victory-gpu/src/watch.rs` `rest` |
| `VICTORY_GUARD_ASK` | off | `1` only | `guard` asks Windows every time, as before. | `crates/victory-gpu/src/watch.rs` `guard` |
| `VICTORY_GUARD_HOLES` | on | `0` | `is_view` looks past unmapped first pages; `0`: first address only. | `crates/victory-gpu/src/watch.rs` `is_view` |
| `VICTORY_PAGE_WRITES` | bit 1 (also `guards`) | `0` count only, `1` = bits 1+2, `slots` = bit 2, `all` = 7 | What is done with writes the guards do not see (DirectStorage): open guarded blocks (1), make texture slots due (2), plain reads too (4). | `crates/victory-gpu/src/watch.rs` `page_writes` |
| `VICTORY_COPY_BESIDE` | off | `1` only (`asked_for!`) | The title's `memcpy`/`memmove`/`memset` are ours and write beside the guards. **[A/B]** | `crates/victory-gpu/src/titlewrite.rs` `copies_on`, `import` |
| `VICTORY_STORE_BESIDE` | off | `1` only (`asked_for!`) | A faulting store is done by us and the thread goes on. **[A/B]** | `crates/victory-gpu/src/titlewrite.rs` `stores_on` |
| `VICTORY_SAME_RECORDS` | on (`toggled!`) | `0` | Descriptor bytes already there are not written again. **[A/B]** | `crates/victory-gpu/src/titlewrite.rs` `put` |
| `VICTORY_UPLOAD_LOW` | on | `0` | `allocate_upload` memory comes from arenas between 4 GiB and 2^37; `0`: the process heap. | `crates/victory-gpu/src/upload.rs` `low` |
| `VICTORY_INPLACE` | 0 | 0, 1, 2 | Title memory opened as a host heap instead of copied: 1 windows in private memory, 2 title buffers too (not with late wait). | `crates/victory-gpu/src/inplace.rs` `level` |
| `VICTORY_INPLACE_MB` | 4096 | MB | Most title memory in place at once. | `crates/victory-gpu/src/inplace.rs` `most` |
| `VICTORY_INPLACE_PROBE` | 0 | 0, 1, 2 | Probe of the driver's in-place openings and a census (2 opens part of the title's memory once). | `crates/victory-gpu/src/inplace.rs` `probe_level` |
| `VICTORY_INPLACE_PROBE_MB` | 64 (at most 1024) | MB | Size of the probe's memory. | `crates/victory-gpu/src/inplace.rs` `tick` |
| `VICTORY_PICKED_WRITES` | 0 | 0, 1, 2 | Buffers written through heap-picked views: declared (1), landed too (2). | `crates/victory-gpu/src/picked.rs` `stage` |
| `VICTORY_PICKED_MB` | 32 | MB | Bytes looked at in turn per frame for those. | `crates/victory-gpu/src/picked.rs` `slice` |
| `VICTORY_HEAP_SYNC` | on | `0`; `all` | Descriptor records the title copies as bytes are mirrored (`sync_heaps`); `all`: every new record at once, not left to draws. | `crates/victory-gpu/src/descriptor.rs` `sync_on`; `crates/victory-gpu/src/descriptor/host.rs` `copied_waits` |
| `VICTORY_HEAP_SYNC_ASIDE` | on (needs heap sync) | `0` | `sync_heaps` runs on a thread of its own beside preparation. | `crates/victory-gpu/src/descriptor.rs` `sync_aside` |
| `VICTORY_HEAP_SYNC_EARLY` | 4 ms | ms; `0` none | How long that thread waits for a submission before a round of its own. | `crates/victory-gpu/src/descriptor.rs` `sync_early` |
| `VICTORY_VIEW_INLINE` | on | `0` | Constant-buffer and placed views are named whole in their records, not table entries. | `crates/victory-gpu/src/descriptor.rs` `inline` |
| `VICTORY_WRITE_SKIP` | on | `0` | A UAV buffer range is declared written only if the shader can store to buffers. | `crates/victory-gpu/src/draw.rs` `write_skip` |
| `VICTORY_DECLARE_ONCE` | on | `0` | Shader-write ranges are declared once until the list next writes title memory. | `crates/victory-gpu/src/host.rs` `declare_once` |
| `VICTORY_UNWATCHED_FRAMES` | 1 | frames | Interval at which copies without write tracking are refreshed whole by the heap buffer pass. | `crates/victory-gpu/src/draw.rs` `heap_buffer_pass` |
| `VICTORY_CARD_BUFFERS` | on | `0` | Copies of big title buffers live in card memory (read when a copy is made). | `crates/victory-gpu/src/shadow.rs` `cards` |
| `VICTORY_CARD_BUFFERS_MIN_MB` | by VRAM budget: 16 (<= 12 GiB), 4 (16 GB), 1 (>= 20 GiB) | MB | Smallest title buffer whose copy goes to card memory. | `crates/victory-gpu/src/shadow.rs` `card_limits` / `resolve_card_limits` |
| `VICTORY_CARD_BUFFERS_MB` | by VRAM budget: 512, 2048, up to 8192 | MB | Most bytes of such copies. | `crates/victory-gpu/src/shadow.rs` `card_limits` |
| `VICTORY_CARD_BUFFERS_MIN_KB` | unset | KiB > 0 | Overrides the minimum in KiB (below 1 MiB). | `crates/victory-gpu/src/shadow.rs` `card_limits` |
| `VICTORY_CARD_ROOM_FOR_BUFFERS` | on | `0` | The texture queue keeps room for card copies of buffers not made yet. (The comment's first line says "off by default"; the code and a later line say on.) | `crates/victory-gpu/src/shadow.rs` `card_room_kept_on` |
| `VICTORY_CARD_ALL` | off | `1` only | Every title buffer copy has two sides (card and PC), however small. | `crates/victory-gpu/src/shadow.rs` `card_all` |
| `VICTORY_CARD_WINDOWS_MB` | 0 | MB | Bytes of windows outside buffers whose copies go to card memory. | `crates/victory-gpu/src/shadow.rs` `window_card_most` |
| `VICTORY_WINDOWS_BAR_MB` | 0 | MB | Bytes of windows in card memory filled by the CPU over the bus (`GPU_UPLOAD` heap). | `crates/victory-gpu/src/shadow.rs` `window_mapped_most` |
| `VICTORY_WINDOWS_OWN` | off | `1` only | Windows outside buffers have two sides too. | `crates/victory-gpu/src/shadow.rs` `windows_own` |
| `VICTORY_WINDOW_KB` | 4096 | KiB, power of two >= 64 | Grid that windows of asks with a length are cut on. | `crates/victory-gpu/src/shadow.rs` `window` |
| `VICTORY_WINDOW_TAILS` | off | `1` only | A window of an ask without a length begins at the ask (pays with `VICTORY_WINDOW_KB` < 4096). | `crates/victory-gpu/src/shadow.rs` `tails` |
| `VICTORY_MEMORY_CARD_MB` | 4096 | MB | Most card memory for the by-address side of title memory. | `crates/victory-gpu/src/shadow.rs` `by_address` |
| `VICTORY_SNAPSHOT_POOL` | 1024 | count (8 was the old value) | Snapshot buffers kept for reuse. | `crates/victory-gpu/src/shadow.rs` `snapshots_kept` |
| `VICTORY_WIDE_BACK` | on | `0` | Snapshots and compares of shader-written ranges run on several threads. (The comment says "off by default"; the code is `is_none_or(!= "0")`.) **[play: 1]** | `crates/victory-gpu/src/shadow.rs` `wide_back` |
| `VICTORY_WIDE_SPIN` | 0 | microseconds | Spin before sleeping in those waits. | `crates/victory-gpu/src/shadow.rs` `spin` |
| `VICTORY_KEEP_SNAPSHOTS` | off | `1` only | A copy's snapshot is kept between submissions (turns the relay off). | `crates/victory-gpu/src/shadow.rs` `keeps` |
| `VICTORY_KEEP_SNAPSHOTS_MB` | 256 | MB | Most bytes of kept snapshots. | `crates/victory-gpu/src/shadow.rs` `kept_most` |
| `VICTORY_LAND_ECHO` | on (`toggled!`) | `0` | A refresh that finds only a landing's bytes sends nothing up. **[A/B]** **[play -Safe: 0]** | `crates/victory-gpu/src/shadow.rs` `land_echo` |
| `VICTORY_SEND_CHANGED` | off | not `0` | A card copy's refresh sends only the dwords title memory changed (off since a bad run). Also read by two tests in `host.rs`. | `crates/victory-gpu/src/shadow.rs` `send_changed` |
| `VICTORY_EXACT_BACK` | on | `0` | Shader writes go into title memory run by run, not first-to-last changed byte. | `crates/victory-gpu/src/shadow.rs` `exact_back` |
| `VICTORY_UNDER_WIDE` | on (`toggled!`) | `0` | Pages of `merge_under_writes` are compared on several threads. **[A/B]** **[play -Safe: 0]** | `crates/victory-gpu/src/shadow.rs` `under_wide` |
| `VICTORY_CARD_HASH_LEN` | on | `0` | The card's hashes cover the title buffer's own bytes, not the 64 KiB-rounded buffer. | `crates/victory-gpu/src/shadow.rs` `Shadow::card_hash_size` |
| `VICTORY_QUICK_FIND` | on (`toggled!`) | `0` | The `FOUND` lookup cache is kept. **[A/B]** | `crates/victory-gpu/src/shadow.rs` `quick_find` |
| `VICTORY_CARD_REHASH_RUNS` | off | `1` only | Pieces hashed again after sends are hashed in runs of touching pieces. **[play: 1]** | `crates/victory-gpu/src/host.rs` `rehash_runs` |
| `VICTORY_REFRESH_CHECK` | off | `1` only | Diagnostic: samples 256 bytes per 64 KiB after a trusted refresh and reports stale copies (about 100 ms a frame). | `crates/victory-gpu/src/shadow.rs` `refresh_check` |

## Textures

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_HEAP_TEXTURES` | on | `0` | Textures shaders pick by computed index are created and uploaded from the view heap. | `crates/victory-gpu/src/draw.rs` `State::heap_textures` |
| `VICTORY_TEXTURE_STAGE_MB` | 96/192/256/384 MB by VRAM budget (`staging_budget_per_frame`) | MB | Bytes per frame that pass may stage. | `crates/victory-gpu/src/draw.rs` `State::heap_textures` |
| `VICTORY_TEXTURE_STAGE_MS` | 250 | ms | Time per pass for making and staging. | `crates/victory-gpu/src/draw.rs` `State::heap_textures` |
| `VICTORY_TEXTURE_CATCHUP` | on | `0` | A backlog (1024+ due) gets three times the bytes and time. | `crates/victory-gpu/src/draw.rs` `State::heap_textures` |
| `VICTORY_EARLY_TEXTURES_MS` | 12 | ms; `0` never | Per-frame budget of that pass before an index-picking shader asks. | `crates/victory-gpu/src/draw.rs` `early_budgets` |
| `VICTORY_BAKE_TEXTURES_MS` | 1500 | ms; `0` nothing special | Budget just before a draw of the title's decoration bake. | `crates/victory-gpu/src/draw.rs` `early_budgets` |
| `VICTORY_BAKE_EXACT` | on | `0` | A bake draw sees picked textures as title memory has them at that moment. | `crates/victory-gpu/src/draw.rs` `bake_exact` |
| `VICTORY_RECHECK_SOON` | on | `0` (after four frames) | A texture found changed with nothing to tell of it is looked at again next frame. | `crates/victory-gpu/src/draw.rs` `recheck_soon` |
| `VICTORY_HEAP_SPREAD` | on | `0` | The no-write rechecks are spread over passes (each slot every 16th). | `crates/victory-gpu/src/draw.rs` `heap_spread` |
| `VICTORY_FULL_CARD_PASS` | on | `0` | On a full card, slots needing a new host copy are held back (about 4 s or until room). | `crates/victory-gpu/src/draw.rs` `full_card_pass` |
| `VICTORY_REUPLOAD_WHEN_FULL` | on | `0` | A texture that has its host copy is still updated while the card is full. | `crates/victory-gpu/src/draw.rs` `reupload_when_full` |
| `VICTORY_HEAP_RECHECK` | on | `0` | Index-picked textures are looked at again a few times after upload. | `crates/victory-gpu/src/draw.rs` `heap_recheck` |
| `VICTORY_TEXTURE_POOL` | on | `0` | Textures the title fills, and their staging, are cut from blocks. | `crates/victory-gpu/src/pool.rs` `on` |
| `VICTORY_RIPE_BLOCKS` | 12 | count; `0` none | Texture blocks made ahead, so one has existed for a while before use. | `crates/victory-gpu/src/pool.rs` `ahead` |
| `VICTORY_TEXTURE_KEEP` | idle 10 min under 88 % of budget, else down to 85 % | `0`: `vram_idle_ms`' older rule (5 s); `1`: older rule with its longer times | When idle host copies of title-filled textures are let go. | `crates/victory-gpu/src/host.rs` `vram_idle_ms`; `crates/victory-gpu/src/resource.rs` `let_go_idle` |
| `VICTORY_TEXTURE_SWEEP` | on, in parts of 8192 | `0` none; `whole` | Letting go of idle host textures. | `crates/victory-gpu/src/resource.rs` `let_go_idle` |
| `VICTORY_TEXTURE_BUDGET_MB` | none (share of the card's budget) | MB | A byte limit on host copies made for index-picked textures. | `crates/victory-gpu/src/resource.rs` `room` |
| `VICTORY_CARD_ROOM_FOR_TARGETS_MB` | 3072 (at most 8192) | MB; `0` none | Card memory left free for render targets not made yet. | `crates/victory-gpu/src/resource.rs` `targets_room_kept` |
| `VICTORY_TEXTURE_ROOM_PERCENT` | 90 | 50-97 | Share of the card's budget up to which such copies are made. | `crates/victory-gpu/src/resource.rs` `room_percent` |
| `VICTORY_LAZY_TEXTURES` | on | `0` | Host copies are made on first use, not at creation. | `crates/victory-gpu/src/resource.rs` `lazy` |
| `VICTORY_HELD_LEVELS` | on (off when levels-on-demand is wanted) | `0` | A placed texture is kept from the first level its views show. | `crates/victory-gpu/src/resource.rs` `held_only` |
| `VICTORY_DEPTH_TWINS` | on | `0`, `stencil`, `nodepth` | Colour twins for depth textures with unordered access (0 none; stencil formats only; no twin for depth planes). | `crates/victory-gpu/src/resource.rs` `twin_texture` |
| `VICTORY_ALIAS_COPY` | on, all shapes | `0` none; `1` one level and layer only | Textures viewed at another texture's address in another format are found and copied between. | `crates/victory-gpu/src/resource.rs` `alias_of` |
| `VICTORY_PLACED_FILLER` | off | `1` only | Filler words of placed texture descriptions stay in the key (twins as before). | `crates/victory-gpu/src/resource.rs` `texture_at` |
| `VICTORY_COPY_OVER_TITLE` | 2: the first copy into a title texture writes zeros where it does not copy | `0`; `title` | `title`: what title memory holds instead; `0`: neither (what exactly `0` restores: not verified). | `crates/victory-gpu/src/list.rs` `copy_texture_region` |
| `VICTORY_READ_BACK` | on | `0` | Texture-to-buffer read backs are made (texels go to the title's buffer). | `crates/victory-gpu/src/list.rs` `read_back` |
| `VICTORY_TEXTURE_BACK` | the four water formats (also for `1`) | `0` none; `<w>x<h>f<format>,...` | Textures shaders write that the CPU reads are copied back into title memory. | `crates/victory-gpu/src/texback.rs` `kinds` |
| `VICTORY_TEXTURE_HOT` | on | `0` | A guarded texture that changed unseen is hashed at every check for a while. | `crates/victory-gpu/src/texture.rs` `hot_textures` |
| `VICTORY_TEXTURE_NO_WAIT` | off | `1` only | A texture refresh does not wait while the runtime remaps title memory. **[play: 1]** | `crates/victory-gpu/src/texture.rs` `no_wait` |
| `VICTORY_TEXTURE_FILL` | on | `0` | With no-wait, a never-filled staging buffer is filled from the pages that are there. | `crates/victory-gpu/src/texture.rs` `fill_unlocked` |
| `VICTORY_TEXTURE_COPY` | off | `1` only | A fully mapped texture is read through the kernel's copy, not in place. | `crates/victory-gpu/src/texture.rs` `through_copy` |
| `VICTORY_TEXTURE_ONCE` | on | `0` | A one-upload staging buffer is filled without asking the write watch and guards. | `crates/victory-gpu/src/texture.rs` `once` |
| `VICTORY_TEXTURE_GUARD` | on | `0` | Textures in view memory are guarded and hashed every eighth frame, not every check. | `crates/victory-gpu/src/texture.rs` `refresh_timed` |
| `VICTORY_TEXTURE_SAME` | on | `0` | An upload is left out when the host copy holds those bytes already. | `crates/victory-gpu/src/texture.rs` `unchanged` |
| `VICTORY_MAPPED_BY_VIEWS` | on | `0` | Mapped bytes of a streamed texture come from the runtime's views, not the host. | `crates/victory-gpu/src/texture.rs` `mapped_bytes` |
| `VICTORY_MAPPED_BY_PAGE` | off (by 64 KiB block) | `1` only | Mapped bytes counted page by page. | `crates/victory-gpu/src/texture.rs` `mapped_bytes` |
| `VICTORY_LEVELS_ON_DEMAND` | off | `1` only (needs tiled resources tier 2) | Host copies of title textures take card memory level by level. | `crates/victory-gpu/src/tiles.rs` `wanted` |
| `VICTORY_LEVELS_MIN_KB` | 1 MB | KiB | Smallest texture made that way. | `crates/victory-gpu/src/tiles.rs` `floor` |
| `VICTORY_UPLOAD_AHEAD` | off | not `0` | First uploads of index-picked textures go on a copy queue of their own. **[A/B]** **[play: 1, not with -Safe]** | `crates/victory-gpu/src/texture/ahead.rs` `on` |
| `VICTORY_SHOW_AFTER` | off | not `0` | A slot shows a texture new to it only after its upload's submission finished. **[play: 1]** | `crates/victory-gpu/src/descriptor/host.rs` `show_after` |
| `VICTORY_STALE_TEXTURES` | start rule | `0` never; `overlap` | When a written view makes older views of other memory stale. | `crates/victory-gpu/src/descriptor/host.rs` `stale_rule` |
| `VICTORY_TABLED_TEXTURES` | off (tabled slots leave the queue) | `1` only | `1` keeps slots read through tables in the index-picked queue. | `crates/victory-gpu/src/descriptor/host.rs` `tabled_out` |
| `VICTORY_TEXTURE_AWAY_MS` | 3000 | ms; `0` at once | How long an unmapped texture's slot waits before it is made null. | `crates/victory-gpu/src/descriptor/host.rs` `HostHeap::texture_slot_gone` |
| `VICTORY_KEEP_VIEWS` | on | `0` | A slot rewritten with another view of its placed texture keeps the old until the new is made. | `crates/victory-gpu/src/descriptor/host.rs` `keep_views` |
| `VICTORY_KEEP_SHOWN` | 1 | 0, 1, 2 | A slot rewritten with another texture keeps showing the old one: 0 never, 1 same kind, 2 same dimension. **[A/B]** (on = 2, off = 0) | `crates/victory-gpu/src/descriptor/host.rs` `keep_shown` |

## Draws, indirect and pipelines

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_INDIRECT_WINDOW` | off | `1` only | Indirect draws reading consecutive records are clamped a window at a time. **[play: 1]** | `crates/victory-gpu/src/draw.rs` `windows` |
| `VICTORY_INDIRECT_BATCH` | 1 | `0`, `1`, `2` | Indirect executions of one record are clamped in batches; 2: a dispatch ends a batch only at a wait or draw. **[play: 2]** | `crates/victory-gpu/src/draw.rs` `batches` |
| `VICTORY_INDIRECT_WIDE` | on (AMD only: 2^20 groups) | `0` | Most groups in x an indirect dispatch runs; `0`: 65,535. | `crates/victory-gpu/src/indirect.rs` `widest` |
| `VICTORY_INDIRECT_REST` | on | `0` | A record that does not fit the budget takes the rest (later ones run empty). | `crates/victory-gpu/src/indirect.rs` `rest` |
| `VICTORY_ROOT_VIEW_MB` | no limit | MB | Limit on what a root view is taken to cover. | `crates/victory-gpu/src/draw.rs` `root_view` |
| `VICTORY_ROOT_UAV_MB` | no limit | MB | Limit for a root UAV's view, refresh and copy-back. | `crates/victory-gpu/src/draw.rs` `root_uav` |
| `VICTORY_ROOT_SRV_DIRECT` | off (one-view tables) | any value, even `0` (`is_none`) | Root SRVs and UAVs are bound as root descriptors instead of bounded one-view tables. | `crates/victory-gpu/src/pipeline/host.rs` `bounded_root_srvs` |
| `VICTORY_BIND_CACHE` | on | `0` | The same render targets are not set on the host list again. | `crates/victory-gpu/src/draw.rs` `set_targets` |
| `VICTORY_THREAD_DIMENSIONS` | on | `0` | Dispatch counts with the console's "thread dimensions" bit are taken as threads. | `crates/victory-gpu/src/draw.rs` `thread_dimensions` |
| `VICTORY_READY_ONCE` | on (`toggled!`) | `0` | A table read again with nothing changed in the list is left as it is. **[A/B]** | `crates/victory-gpu/src/draw.rs` `ready_once` |
| `VICTORY_LENT_HEAP` | on | `0` | A list without a view heap borrows the title's biggest; `0`: such a dispatch is dropped. | `crates/victory-gpu/src/descriptor.rs` `lent_heap` |
| `VICTORY_VIEW_BYTES` | on | `0` | Buffer views off their copy's elements get a copy of their own bytes; `0`: null. | `crates/victory-gpu/src/descriptor/host.rs` `view_bytes` |
| `VICTORY_RAW_VIEWS` | on | `0` | View heaps keep a raw twin of every buffer view (needs a 2^22-view heap). | `crates/victory-gpu/src/descriptor/host.rs` `raw_twins` |
| `VICTORY_DYNAMIC_SLOTS` | 2^20 (`DYNAMIC << 2`) | count | Most dynamic slots of a heap where the host has room. | `crates/victory-gpu/src/descriptor/host.rs` `dynamic_slots` |
| `VICTORY_SLOT_HOLDS` | on | `0` | A slot holds the constant buffer it views, not only the list. | `crates/victory-gpu/src/descriptor/host.rs` `HostHeap::constant_at` |
| `VICTORY_VIEW_COPY` | on | `0` only | Descriptors are made in thread-private heaps and copied whole; `0`: made in place. | `crates/victory-gpu/src/descriptor/host.rs` `staged` |
| `VICTORY_VIEW_EXTENT` | on | `0` | A buffer view is updated in the host copy its slot shows. | `crates/victory-gpu/src/descriptor/host.rs` `view_extent` |
| `VICTORY_QUICK_MIRROR` | on | `0` | A slot already made from its record is told so without the lock. | `crates/victory-gpu/src/descriptor/host.rs` `quick_mirror` |
| `VICTORY_LIST_VIEWS` | on (`toggled!`) | `0` | A list makes a view its tables hold once, not per table. **[A/B]** | `crates/victory-gpu/src/descriptor/host.rs` `list_views` |
| `VICTORY_SCISSOR_TEST` | off | `full` | Experiment: every scissor rectangle widened to the whole target. | `crates/victory-gpu/src/draw.rs` `scissor_test` |
| `VICTORY_MESH_TEST` | off | `skip`, `scissor`, `keep:<hex>[:<hex>]` | Experiment: mesh draws dropped, rasterised to one pixel, or kept only for matching shader hashes. | `crates/victory-gpu/src/draw.rs` `mesh_test`, `mesh_kept` |
| `VICTORY_PIPELINE_BOOK` | off | `1` only | Every pipeline the title creates is written to disk, for `victory warm`. | `crates/victory-gpu/src/pipeline/known.rs` `on` |
| `VICTORY_EARLY_ALL` | off | `1` only | Every title pipeline is started at creation, requested before or not. | `crates/victory-gpu/src/pipeline/library.rs` `wanted_early` |
| `VICTORY_LIBRARY_LOCK` | off | `1` only | One pipeline-library call at a time, as before. | `crates/victory-gpu/src/pipeline/library.rs` `one_call` |
| `VICTORY_LIBRARY_MAP` | on | `0` | The library file is mapped, and saved through a mapping. | `crates/victory-gpu/src/pipeline/library.rs` `mapped` |
| `VICTORY_LIBRARY_HANDLE` | on | `0` | The mapped library file's handle stays open (closing it caused zero pages after a save). | `crates/victory-gpu/src/pipeline/library.rs` `holds` |
| `VICTORY_LIBRARY_TRIM` | on | `0` | Library pages are let go once nothing loaded for ten seconds. | `crates/victory-gpu/src/pipeline/library.rs` `trim` |
| `VICTORY_PIPELINE_WAIT_MS` | 500 | ms | A frame's budget for waiting on pipelines being made. | `crates/victory-gpu/src/pipeline/make.rs` `wait_budget` |
| `VICTORY_SCREEN_WAIT_MS` | 10000 | ms; `0` like the others | The budget for passes over the screen. | `crates/victory-gpu/src/pipeline/make.rs` `screen_budget` |
| `VICTORY_PIPELINE_WAIT_SHARED` | on | `0` | The frame's budget is shared by all recording threads. | `crates/victory-gpu/src/pipeline/make.rs` `shared_budget` |
| `VICTORY_PIPELINE_SLOW` | on (a quarter of workers) | `0` only | Limits workers on known-slow pipelines no draw asked for. | `crates/victory-gpu/src/pipeline/make.rs` `slow_limit` |
| `VICTORY_PIPELINE_WORKERS` | two thirds of logical CPUs, 2-16 | 1-64 | Number of pipeline workers. | `crates/victory-gpu/src/pipeline/make.rs` `workers` / `worker_count` |
| `VICTORY_PIPELINE_LOWEST` | on | `0` | Workers run below the title's threads (lowest priority). | `crates/victory-gpu/src/pipeline/make.rs` `workers` |
| `VICTORY_SKIP_PIPELINES` **[diag]** | none | 16-digit shader hashes, comma-separated | Pipelines using those shaders are skipped. **[play: none]** (`none` is not a hash: nothing is skipped and a warning is logged.) | `crates/victory-gpu/src/pipeline.rs` `skipped_shader` / `parse_skip_hashes` |

## Shaders (the translator)

`VICTORY_SHADER_OPT` is a bit mask of the clean-up passes in
`crates/victory-shader/src/emit/bitcode/opt.rs`. Every pass is exact. Use the
mask to tell a translator fault from a clean-up fault, and to measure what
the clean-up is worth.

| Bit | Name | Pass |
|---|---|---|
| 1 | `DEAD` | dead code |
| 2 | `FOLD` | constants and identities |
| 4 | `REUSE` | one instruction for one value |
| 8 | `FACTS` | what a taken branch says about its condition |
| 16 | `LOADS` | one read for one place within a block |
| 32 | `FIRST` | a first-lane read of two host waves without the half-has-a-lane round |
| 64 | `WIDE` | the dwords of one console load read by one call for every four |

Unset or unparsable: `DEFAULT` = 63 (everything except `WIDE`). `0`: no passes.
`127`: every pass. The value is masked with `ALL` (127) and accepts `0x` hex.
Parsed by `emit::parse_shader_opt`.

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_SHADER_OPT` | 63 | bit mask above | Which clean-up passes run over translated code. **[key]** | `crates/victory-shader/src/emit/mod.rs` `shader_opt` |
| `VICTORY_SHADER_LOOP_LIMIT` **[diag]** | `DEFAULT_LOOP_LIMIT` (`lower::LOOP_LIMIT`) | number >= 1, decimal or `0x` | Iterations after which a translated loop is left; a warning is logged when changed. **[key]** | `crates/victory-shader/src/emit/mod.rs` `loop_limit` |
| `VICTORY_GROUP_SYNC` | 2 (every barrier waits) | `0`, `1`, `2` | 1: the title's `s_barrier` waits only where every lane runs; 0: no barrier waits (wrong values, for diagnosis). **[key]** | `crates/victory-shader/src/emit/mod.rs` `group_sync` |
| `VICTORY_RDNA_FLOAT` | all bits (15) | bit set: 1 `MAX_MIN`, 2 `LEGACY`, 4 `MED3`, 8 `FRACT`; `0` none | On an AMD RDNA host, which float instructions are written as the single host operation. **[key]** (via `host_settings`) | `crates/victory-shader/src/emit/mod.rs` `rdna_float` |
| `VICTORY_HELPER_OWN` | on | `0` | A pixel shader's helper lane reads its own value where the console reads the first lane's. **[key]** | `crates/victory-shader/src/emit/mod.rs` `helper_own` |
| `VICTORY_F16_NEAREST` | off | `1` only | 16-bit float results rounded to nearest as on the console (about a dozen instructions each). **[key]** | `crates/victory-shader/src/emit/lower.rs` `f16_nearest_on` |
| `VICTORY_WAVE_WIDE` | the barrier form off; the 32-lane widening on | `1` turns the barrier form on; `0` turns the widening off | Forms of a wave64 program run as two host waves. **[key]** | `crates/victory-shader/src/emit/mod.rs` `emit` |
| `VICTORY_WAVE_DEEP_SKIPS` | on | `0` | The form with every invisible skip left out (`ir::without_skips`). **[key]** | `crates/victory-shader/src/emit/mod.rs` `emit` |
| `VICTORY_WAVE_LOCAL` | on | `0`; `idiom` | Each host wave as a wave of its own; `idiom`: only where exact. **[key]** | `crates/victory-shader/src/emit/mod.rs` `emit` |
| `VICTORY_WAVE_OWN_BITS` | on | `0` | A wave OR where not every lane runs is translated by the own-bits rule (`emit/own_bits.rs`). **[key]** | `crates/victory-shader/src/emit/lower.rs` `own_bits_on` |
| `VICTORY_FIRSTLANE_OWN` | off | `1` only | A first-lane read takes the host wave's own first lane. **[key]** | `crates/victory-shader/src/emit/lower.rs` `valu_op` (first-lane case) |
| `VICTORY_FIRST_HALF` | on | `0` | In a retranslation, first halves of interpolations are evaluated as the pair they start. **[key]** | `crates/victory-shader/src/emit/lower.rs` `Lower::new` / `with_first_half_pairs` |
| `VICTORY_STRICT_JOINS` | off | any value | Joins read only through other joins are marked too (40 programs then stop). **[key]** | `crates/victory-shader/src/emit/lower.rs` `fill_phis` |
| `VICTORY_MESH_OUTPUT_STRICT` | off | any value | Mesh shaders whose outputs exceed the host's 32 KiB are refused. **[key]** | `crates/victory-shader/src/emit/lower.rs` `mesh_prologue` |
| `VICTORY_WAVE_WHY` | off | any value | Prints why a program keeps 64-lane waves and which forms failed. | `crates/victory-shader/src/emit/mod.rs` `emit`; `crates/victory-shader/src/emit/lower.rs` `Lower::new` |
| `VICTORY_DESC_WHY` | off | any value | Prints `DESC-WHY` lines for descriptor data lowering. | `crates/victory-shader/src/emit/lower.rs` `descriptor_data` |
| `VICTORY_SHADER_SAVE` | off | dir | Every shader saved as the title gave it (`.cdxil`) and as translated (`.dxil`). | `crates/victory-gpu/src/pipeline.rs` `translated` |
| `VICTORY_SHADER_CENSUS` | off | any value; in `translate`, a file appended to | Counts a module's instructions by kind; `translate` writes one line per program. | `crates/victory-shader/src/emit/bitcode/census.rs` `enabled`; `apps/victory/src/translate.rs` `census_line` |

## Ray tracing and ambient light

See `docs/fork-ray-tracing.md` for the background.

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_AMBIENT` | off (the title's own output) | a number > 0 (share of the estimate); `constant:<level>`; `0`/`off`/garbage = off | Ambient stand-in, estimated from the frame's visible surfaces, when the title's rays find nothing. **[play: 0.8]** | `crates/victory-gpu/src/ambient.rs` `set` / `parse` |
| `VICTORY_AMBIENT_SHADE` | strength 1.0, radius 0.6 m | `<strength>,<radius m>` | Crease shading of the stand-in. | `crates/victory-gpu/src/ambient.rs` `shade` |
| `VICTORY_AMBIENT_AIR` | 0 (as the title has it) | multiple > 0 | Bounds what the air scatters, as a multiple of the light around. | `crates/victory-gpu/src/ambient.rs` `air` |
| `VICTORY_BVH_BUILDER` | off | `1` only | The title's structure builder dispatches may run (all its pipelines made first). **[play -RayTracing: 1]** | `crates/victory-gpu/src/pipeline.rs` `builder_runs` |
| `VICTORY_BVH_QUARANTINE` | off | `1` only | What the builder's stages store stays in host copies, not copied back to title memory. | `crates/victory-gpu/src/pipeline.rs` `builder_quarantined` |
| `VICTORY_BVH_SIZES_WAIT` | off | `1` only | A submission that asks for structure sizes is waited for and landed before the title goes on. | `crates/victory-gpu/src/memory.rs` `sizes_wait` |
| `VICTORY_BVH_FENCES` | on | `0` | The builder's buffer atomics stand between two fences. **[key]** | `crates/victory-shader/src/emit/mod.rs` `builder_fences` |
| `VICTORY_BVH_BOUNDS` | on | `0` | The builder's root-view accesses stay near them; impossible header sizes count as none. **[key]** | `crates/victory-shader/src/emit/mod.rs` `builder_bounds` |
| `VICTORY_BVH_TRACE` **[diag]** | off | `1` only | Logs the sizes the builder is asked for and copies to places of that size. | `crates/victory-gpu/src/memory.rs` (`on` in the trace module) |
| `VICTORY_MEMORY_DUMP` **[diag]** | off | folder | When a file `go` appears there, the arenas are written out with `pages.txt`. | `crates/victory-gpu/src/memory.rs` `folder` |

## Capture, replay, timing and diagnostics

See `docs/replay.md` for the recorder and the replayer.

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_AB` **[diag]** | off | `<seconds>,<switch>[,<switch>..]`, seconds > 1, 1-8 switches | Cycles the named switches through all combinations, one phase per period, and logs each phase's frame times. | `crates/victory-gpu/src/ab.rs` `asked` / `parse` |
| `VICTORY_CAPTURE` **[diag]** | off | folder | Presented frames numbered by powers of two saved as half-size PNGs. | `crates/victory-gpu/src/capture.rs` `folder` |
| `VICTORY_CAPTURE_FRAMES` **[diag]** | none | frame numbers, comma-separated | Those frames saved too, their draws traced at debug level. | `crates/victory-gpu/src/capture.rs` `listed` |
| `VICTORY_CAPTURE_WORLD` **[diag]** | off | any value, even `0` (`is_some`) | Also frames 2 and 6 after the first frame with 500+ draws, the 30th and 80th, and a timeline of small pictures. | `crates/victory-gpu/src/capture.rs` `world_on` |
| `VICTORY_CAPTURE_WORLD_ALSO` **[diag]** | none | frame offsets, comma-separated | More frames counted from that first world frame. | `crates/victory-gpu/src/capture.rs` `listed` |
| `VICTORY_DUMP_TARGETS` **[diag]** | off | `switch::on`; needs `VICTORY_CAPTURE` | Textures the listed frames render to or write are saved as PNGs. | `crates/victory-gpu/src/capture.rs` `dump_on` |
| `VICTORY_RECORD` **[diag]** | off (nothing hooked) | folder; empty = off | The frame recorder: captures go to `capture-<n>` in it. | `crates/victory-gpu/src/replay/record.rs` `Settings::from_env` |
| `VICTORY_RECORD_AT` **[diag]** | trigger: a file `go` in the folder | seconds, or `f<frame>` | When the capture is taken. | `crates/victory-gpu/src/replay/record.rs` `Settings::from_env` / `parse_at` |
| `VICTORY_RECORD_FRAMES` **[diag]** | 1 | count | Frames captured. | `crates/victory-gpu/src/replay/record.rs` `Settings::from_env` |
| `VICTORY_RECORD_ALWAYS` **[diag]** | off | `1` only | Lists are recorded from the start, not only near a capture. | `crates/victory-gpu/src/replay/record.rs` `Settings::from_env` |
| `VICTORY_REPLAY_GBV` **[diag]** | off | any value | GPU-based validation on the replayer's device (very slow). | `crates/victory-gpu/src/replay/play.rs` `make_device` |
| `VICTORY_REPLAY_PEEK` **[diag]** | off | any value | The divergence search peeks at every draw, not only where kinds differ. | `crates/victory-gpu/src/replay/play/diverge.rs` `diverge_draws` |
| `VICTORY_TIMELINE` **[diag]** | off | `<file>`, or `<from s>[+<from s>..],<frames>,<file>` | A Chrome-trace timeline of a few frames plus a text summary. | `crates/victory-gpu/src/timeline/record.rs` `asked` |
| `VICTORY_TIMELINE_AT` **[diag]** | 60 | seconds | Start of the single timeline. | `crates/victory-gpu/src/timeline/record.rs` `asked` |
| `VICTORY_TIMELINE_FRAMES` **[diag]** | 3 | count | Frames in the single timeline. | `crates/victory-gpu/src/timeline/record.rs` `asked` |
| `VICTORY_GPU_TIMING` **[diag]** for the timing | off | `switch::on` for the timing; **any value** turns late wait off in every build | Each list runs and is timed on its own, slow ones logged. | `crates/victory-gpu/src/host.rs` `timing`, `late_stage` |
| `VICTORY_CARD_SPANS` **[diag]** | off | `1` only | The card's own timestamps of our submissions. | `crates/victory-gpu/src/host/spans.rs` `on` |
| `VICTORY_PANEL` **[diag]** | off | `switch::on` | The run's numbers drawn on the picture; F3 shows and hides it. | `crates/victory-gpu/src/host/panel.rs` `asked` |
| `VICTORY_PROFILE` **[diag]** | off | `<from>,<to>,<ms>,<file>` (several with `;`) | Samples the busiest threads' stacks between two times and writes counts. | `crates/victory-loader/src/profile.rs` `profile_on_schedule` |
| `VICTORY_STACKS` **[diag]** | off | seconds, comma-separated | Prints every thread's stack at those times. | `crates/victory-loader/src/stacks.rs` `dump_stacks_on_schedule` |
| `VICTORY_D3D12_DEBUG` **[diag]** | off | `switch::on` | The D3D12 debug layer; host failures name the command. In `victory-shader`'s test host, any value. | `crates/victory-gpu/src/host.rs` `Host::create`; `crates/victory-shader/src/emit/host.rs` `Host::new` |
| `VICTORY_D3D12_GBV` **[diag]** | off | `switch::on`, with the debug layer | GPU-based validation without state tracking. | `crates/victory-gpu/src/host.rs` `Host::create` |
| `VICTORY_D3D12_WARP` **[diag]** | off | `switch::on` | The host device on the software adapter (WARP). | `crates/victory-gpu/src/host.rs` `Host::create` |
| `VICTORY_D3D12_DRED` **[diag]** | off | `light`, `faults`, else any value not `0` = full | Device-removed data: full (markers and texts), `light` (breadcrumbs, faulting address, pipelines), `faults` (address only). **[play: light]** | `crates/victory-gpu/src/host.rs` `dred_on`, `dred_light`, `dred_faults` |
| `VICTORY_TRAIL` | on | `0` | Recent host-object activity is kept and logged at a device removal. | `crates/victory-gpu/src/host.rs` `trail_on` |
| `VICTORY_BOOK` **[diag]** | on, when DRED or the slot check is on | `0` | Book of who owned which card memory, reported at a device loss. | `crates/victory-gpu/src/book.rs` `on` |
| `VICTORY_SLOT_CHECK` **[diag]** | off | not `0` | A thread walks the view heap for slots naming freed memory. | `crates/victory-gpu/src/book.rs` `checks` |
| `VICTORY_FAULT_BOOK` **[diag]** | off | file; empty = off | Book of write faults on guarded view memory for 40 frames from each time. | `crates/victory-gpu/src/faultbook.rs` `frame` |
| `VICTORY_FAULT_BOOK_AT` **[diag]** | `150,210,270` | seconds, comma-separated | When the fault book records. | `crates/victory-gpu/src/faultbook.rs` `frame` |
| `VICTORY_POP_WATCH` **[diag]** | off | `<from>-<to>,...` seconds | Presented frames compared block by block for popping textures. | `crates/victory-gpu/src/popwatch.rs` `asked` |
| `VICTORY_POP_FOLDER` **[diag]** | off | folder | Saves the three pictures around a frame with blips (at most 200). | `crates/victory-gpu/src/popwatch.rs` `folder` |
| `VICTORY_POP_ALL` **[diag]** | off | `1` only | Every watched frame's picture saved. | `crates/victory-gpu/src/popwatch.rs` `watch` |
| `VICTORY_COPY_MODE` **[diag]** | guarded copies | `plain`, `none` | Timing only: copies without the mapped-page guard, or none at all. | `crates/victory-gpu/src/copystat.rs` `mode` |
| `VICTORY_BARRIER_TEST` **[diag]** | off | `groups`, `indirect`, `both` | Leaves out the barrier groups' wait for shader writes, the indirect clamp's two waits, or both. | `crates/victory-gpu/src/copystat.rs` `barrier_test` |
| `VICTORY_MEMORY_MAP` **[diag]** | on | `0` | The address-space walk for the memory map line. | `crates/victory-gpu/src/copystat.rs` `memory_map` |
| `VICTORY_CARD_ACCOUNTING` | off | `1` only | A line on where card memory is and what due texture slots need. | `crates/victory-gpu/src/draw.rs` `State::card_accounting` |
| `VICTORY_WALLS_TRACE` | off | `1` or `2` | Log lines on what the title's CPU was given while deciding "in a room"; DirectStorage: 1 failed reads, 2 every read. | `crates/victory-gpu/src/host.rs` `walls_trace`; `crates/victory-hle/src/modules/dstorage.rs` `walls_trace` |
| `VICTORY_UPCOPY_CHECK` **[diag]** | off | `1` only | Checks whether what goes up to a card copy is older than what the card holds. | `crates/victory-gpu/src/host/upcheck.rs` `on` |
| `VICTORY_LIST_SLOTS` **[diag]** | none | pixel shader hash prefixes, comma-separated | Frame listing: the heap slots those draws pick by index. | `crates/victory-gpu/src/draw.rs` `list_slots` |
| `VICTORY_CRASH_SAVE` **[diag]** | off | `<rva>:<len>,...` hex | Those title ranges written to `crash-<rva>.bin` in the cache dir on a crash. | `apps/victory/src/run.rs` `diagnostics` |
| `VICTORY_WATCH` **[diag]** | off | `<rva>,...` hex | Logs the u32 at title+rva whenever it changes (polled every 5 ms). | `apps/victory/src/run.rs` `watch` |
| `VICTORY_POKE` **[diag]** | off | `<rva>:<byte>,...` hex | Writes those bytes twice a second (e.g. a tick flag). | `apps/victory/src/run.rs` `poke` |

## Tests and tooling

`victory translate` and `victory warm` (`apps/victory/src/translate.rs`,
`apps/victory/src/main.rs`), test and benchmark inputs, and variables our own
processes set for their children.

| Switch | Default (unset) | Values | What it changes | Where read |
|---|---|---|---|---|
| `VICTORY_DXIL` | newest Windows SDK's `dxil.dll`, else `LoadLibrary` | path | The DXIL validator DLL (`translate --validate`, tests). | `crates/victory-shader/src/emit/validator.rs` `Validator::default_path` |
| `VICTORY_PC_DXC` | `dxc.exe` beside the SDK's `dxil.dll` | path | The PC compiler for diff tests, the mesh/pixel test in `pipeline/host.rs`, and `dump_dxil`. | `crates/victory-shader/src/emit/diff_tests.rs` `tools`; `crates/victory-shader/src/emit/mod.rs` `dump_dxil`; `crates/victory-gpu/src/pipeline/host.rs` `dxc` (tests) |
| `VICTORY_CONSOLE_DXC` | diff tests skipped | path | The console SDK's `dxc.exe` for differential tests. | `crates/victory-shader/src/emit/diff_tests.rs` `tools`, `compile` |
| `VICTORY_WARP` | the default adapter | `1` only | Translator test host and diff tests on WARP. | `crates/victory-shader/src/emit/host.rs` `Host::new`; `crates/victory-shader/src/emit/diff_tests.rs` |
| `VICTORY_DIFF_ONLY` | all cases | case name | Runs only that differential test case. | `crates/victory-shader/src/emit/diff_tests.rs` |
| `VICTORY_PROBE` | test returns | folder of `.dxil` | `--ignored probe`: runs each program on the card. | `crates/victory-shader/src/emit/probe_tests.rs` `probe` |
| `VICTORY_REF_PS` | built-in shader | `.dxil` file | Pixel shader for the stub pipeline test. | `crates/victory-shader/src/emit/mod.rs` `stub_pipeline_is_created` (test) |
| `VICTORY_REF_VS` | `stub_vs` | `.dxil` file | Vertex shader for the same test. | `crates/victory-shader/src/emit/mod.rs` `stub_pipeline_is_created` (test) |
| `VICTORY_EMIT_DUMP` | off | dir (refused inside the repository) | `translate --list`: every emitted shader as a file, plus validator failures; diff tests dump their inputs too. | `apps/victory/src/translate.rs` `run`, `check`; `crates/victory-shader/src/emit/diff_tests.rs` |
| `VICTORY_PAIRS` | none | `vs+ps,...` label substrings | `translate`: builds those pipelines after the walk. | `apps/victory/src/translate.rs` `run` |
| `VICTORY_HOST_MAX_WAVE` | (not verified) | lanes, e.g. 32 | `translate`: as for a host whose waves have at most n lanes. | `apps/victory/src/translate.rs` `run` |
| `VICTORY_HOST_RDNA` | off | `1` only | `translate`: as for an AMD RDNA host (see `VICTORY_RDNA_FLOAT`). | `apps/victory/src/translate.rs` `run` |
| `VICTORY_HOST_WAVE` | (not verified) | lanes | `victory translate --warm`: as the run's host (set from the run's log). | `apps/victory/src/main.rs` `main` |
| `VICTORY_VIEW_LIMIT` | (not verified) | null slot | `translate --warm`: as on a host whose view heap is capped. | `apps/victory/src/main.rs` `main` |
| `VICTORY_WARM_SECONDS` | no limit | seconds | `warm` stops taking new shaders after n seconds. | `apps/victory/src/translate.rs` `warm` |
| `VICTORY_TRANSLATE_ONLY` | all | hash prefixes, comma-separated | Translates only those containers. | `apps/victory/src/translate.rs` `translate` |
| `VICTORY_DUMP_MATCH` | off | `<dir>;<text>\|<text>...` | Dumps the containers whose line holds one of the texts. | `apps/victory/src/translate.rs` `translate` |
| `VICTORY_FAIL_DUMP` | off | dir | First 3 containers of each failure reason as `.cdxil`. | `apps/victory/src/translate.rs` `translate` |
| `VICTORY_BCPACK_TABLES` | ignored test panics | file | Format tables for `reference_chunks`. | `crates/victory-formats/src/bcpack.rs` `reference_chunks` (test) |
| `VICTORY_BCPACK_CASES` | ignored test panics | dir | Reference chunks for the same test. | `crates/victory-formats/src/bcpack.rs` `reference_chunks` (test) |
| `VICTORY_XMA_STREAMS` | ignored test panics | dir of `*.xma` + `*.ctx` | Saved streams walked and decoded. | `crates/victory-formats/src/xma.rs` `walks_saved_streams` (test) |
| `VICTORY_XMA_REFERENCE` | ignored test panics | dir | Reference decodes to compare with. | `crates/victory-formats/src/xma.rs` `matches_a_reference_decoder` (test) |
| `VICTORY_BENCH_CS` | our compare shader | HLSL file | Times another source of the card compare. | `crates/victory-gpu/src/indirect.rs` `card_compare_costs` (test) |
| `VICTORY_BENCH_WARP` | the card | any value | Measurement tests run on WARP. | `crates/victory-gpu/src/indirect.rs` (tests, `WarpHarness::on`) |
| `VICTORY_BENCH_S` | 5 or 30 | seconds | Seconds per way of a card measurement. | `crates/victory-gpu/src/indirect.rs` (tests) |
| `VICTORY_BENCH_N` | 3 | count | Runs of each way. | `crates/victory-gpu/src/indirect.rs` `new_blocks_under_readers` (test) |
| `VICTORY_BENCH_THREADS` | 4 | count | Threads that make blocks. | `crates/victory-gpu/src/indirect.rs` `new_blocks` (test) |
| `VICTORY_BLOCK_WAY` | parent process | way name (set by the test for its children) | Runs one way of `new_blocks_under_readers` in a child process. | `crates/victory-gpu/src/indirect.rs` `new_blocks_under_readers` (test) |
| `VICTORY_BENCH_FILE` | texture test: step left out; dstorage test: panics | file | A big file for read benchmarks. | `crates/victory-gpu/src/texture.rs` `cold_texture_costs`; `crates/victory-hle/src/modules/dstorage.rs` `read_bench` (tests) |
| `VICTORY_BENCH_HOLES` | on | `0` | Benchmark views without holes. | `crates/victory-gpu/src/host.rs` `frame_of_refreshes` (test) |
| `VICTORY_TEST_ROUNDS` | 120 | count | Rounds of `landings_title_writes_and_light_refreshes_at_once`. | `crates/victory-gpu/src/host.rs` (test) |
| `VICTORY_TEST_D3D_DEBUG` **[diag]** | off | any value | The recorder/replayer test under the debug layer. | `crates/victory-gpu/tests/replay.rs` `a_frame_is_recorded_and_replayed` |
| `VICTORY_TILES_TEST_CARD` | WARP | `1` only | Levels-on-demand tests run on the card. | `crates/victory-gpu/src/tiles.rs` (tests, `Rig::new`) |
| `VICTORY_SAVE_STOP` | (child marker) | step number | Crash-test child stops for good at that step. | `crates/victory-hle/src/modules/xgamesave.rs` `crash_point` (test) |
| `VICTORY_SAVE_CHILD` | (child marker) | root dir | Marks the crash-test child process. | `crates/victory-hle/src/modules/xgamesave.rs` `crash_child` (test) |
| `VICTORY_SAVE_OP` | (child marker) | operation | The operation the crash-test child runs. | `crates/victory-hle/src/modules/xgamesave.rs` `crash_child` (test) |
| `VICTORY_EXIT_CHILD` | (child marker) | any value | Child of the thread-exit test. | `crates/victory-loader/src/threads.rs` `exit_child` (test) |
| `VICTORY_TRAP_LOG` | (child marker) | file | Child of the trapped-import test; crash log location. | `crates/victory-loader/src/thunks.rs` `trap_child` (test) |
| `VICTORY_HOST_LOCK_CHILD` | (child marker) | set by the test | Loader-lock test child (name in const `CHILD`). | `crates/victory-hle/src/host.rs` (tests) |
| `VICTORY_AUDIO_CAPTURE_OF` **[diag]** | (internal) | process id, set by `capture::start` | Tells the recorder child which process to record (name in const `TARGET`). | `crates/victory-hle/src/modules/audio/capture.rs` `child` |
| `VICTORY_RELAUNCHED_FROM` | (internal) | inherited handle, set by the relaunch | A relaunched `victory` waits for the process it replaces, then removes the variable. | `crates/victory-hle/src/modules/lifecycle.rs` `wait_for_previous` |

## Not switches

The grep matches these tokens, but the code does not read them from the
environment at run time.

| Token | What it is |
|---|---|
| `VICTORY_TRANSLATOR` | Compile-time `rustc-env` set by `crates/victory-gpu/build.rs` (hash of the translator's sources); read with `env!` in `shader_cache.rs` `build`. |
| `VICTORY_SHADER_SOURCE_HASH` | Compile-time `rustc-env` from `crates/victory-shader/build.rs`; read with `env!` in `emit::translation_fingerprint`. |
| `VICTORY_SHADER_SWITCHES` | Compile-time `rustc-env` from `crates/victory-shader/build.rs`: every `"VICTORY_..."` literal in that crate, comma-separated; read with `env!` in `emit::translation_fingerprint`, which has no caller (grep). |
| `VICTORY_A`, `VICTORY_B`, `VICTORY_NONE_SUCH` | Made-up switch names in the unit test of `crates/victory-gpu/src/ab.rs`. |
| the upstream `MERGE_RACE` counter (prefixed name) | Mentioned only in `docs/README-fxceless.md`; nothing in this code reads it, so the full name is left out here (the docs check rejects names no source uses). |
| `VICTORY_READ_WATCH` | Mentioned only in a comment of `crates/victory-gpu/src/texback.rs` (an upstream run); nothing in this code reads it. |
