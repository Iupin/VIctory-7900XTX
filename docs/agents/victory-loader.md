# victory-loader

## Purpose

Runs the title's original x86-64 code inside our process. It prepares a
patched copy of the executable that the Windows loader maps but does not
run, maps it (and the DLLs the title ships), binds every import through an
`ImportResolver`, then starts the title. It also owns what runs under title
code: static-TLS redirection, thread notices for title images, emulation of
console-only CPU instructions, the "look before write" patches, the crash
reporter, and (diagnostics builds) stack dumps and a sampling profiler.
The caller is `apps/victory/src/run.rs` (`run`); the resolver is
`victory_hle::HleResolver` wrapped in `WithTitleDlls`. The import order is in
[docs/architecture.md, "Import resolution"](../architecture.md#import-resolution);
this map does not repeat it.

Most modules are `#[cfg(windows)]`. `prepare`, `resolve` and `publish` are
portable. `profile` and `stacks` exist only with the `diagnostics` feature.

## How it works

### Prepare, map, bind, start (as `apps/victory/src/run.rs` drives it)

1. **Prepare** (`crates/victory-loader/src/prepare.rs`, `prepare(original)`).
   Pure byte patching, testable anywhere. It parses the original with
   `victory_pe::ImageInfo::parse` and keeps that `info`. It copies the bytes
   and changes them:
   - sets `IMAGE_FILE_DLL` and clears the entry point: the OS maps the file
     like a DLL and calls nothing;
   - points the import directory at its own empty terminator descriptor
     (size one descriptor) instead of clearing it. The comment says Windows 10
     assigns static TLS to the DLL only when an empty import table is present.
     The bound-import directory is cleared. The IAT directory is left as is;
   - clears `AddressOfCallBacks` in the TLS directory, and turns that
     field's `DIR64` base relocation into `IMAGE_REL_BASED_ABSOLUTE`, so a
     rebase (Wine always rebases `DYNAMIC_BASE` images) does not turn the 0
     into `delta` (`dir64_relocations`);
   - clears `FORCE_INTEGRITY` and the checksum.

   `PreparedImage { info, bytes, original_hash }`; `original_hash` is BLAKE3 of
   the original file. `PreparedImage::file_name` is
   `victory-image-<first 16 hex digits of the hash>.dll`.

2. **Map** (`crates/victory-loader/src/windows.rs`, `load(prepared, dir)`).
   `dir` must be outside the repository (`victory_core::ensure_outside_repo`);
   the caller passes the cache's `images` folder. The bytes are written there
   with `publish::write_if_changed` (a per-writer staging file, then an atomic
   rename, so two runners cannot clobber each other). Then `LoadLibraryExW`
   maps it as a real module, so debuggers, unwinders and crash dumps see it.
   For an executable (not a DLL) `tls::redirect` runs next (below). Finally
   `threads::mapped` records the image.

   Title DLLs: `load_title_dlls(main, title_dir, cache_dir, wanted)` walks the
   imports and delay imports of `main` recursively (`map_imports`, `map_dll`)
   and maps each DLL found in `title_dir` whose lower-cased name `wanted`
   accepts. Each goes through `prepare` + `load` too. The result is in
   dependency order (dependencies first). `load_title_dll(name, ...)` does
   the same for one DLL the title loads itself (`LoadLibrary`); the CLI wires
   it into `victory_hle::runtime::set_title_dll_loader`. Imports are not bound
   by either function.

3. **Bind** (`LoadedImage::bind_imports(resolver, policy)`). For every entry
   of `info.imports` and `info.delay_imports` it asks `resolver.resolve(dll,
   symbol)`. Delay-load slots are bound eagerly, so the title's own
   delay-load helper never runs. Found addresses are counted by `Provenance`
   (`Hle`, `Host`, `Title`) in a `BindReport`. Each unresolved import gets a
   32-byte stub from `thunks::ThunkBlock` (`mov rcx, &ImportSite; mov rax,
   handler; jmp rax`) whose handler `unimplemented_import` follows
   `UnimplementedPolicy`: `Trap` logs, writes the crash log
   (`crate::log_crash`), breaks into a debugger if one is attached, and
   aborts; `ReturnZero` logs once per import and returns 0. `write_slots`
   unprotects and restores each IAT slot on its own, because import tables can
   sit in sections with different protections (`.didat`).
   `WithTitleDlls { inner, dlls }` is the resolver wrapper: `inner` first,
   then the exports of the mapped title DLLs (`LoadedImage::export_symbol`,
   by name or ordinal through `GetProcAddress`).

   The CLI binds the title DLLs first, then the executable. With
   `--execute` it then applies `write_sites::apply_asked` to the executable
   (below), calls `report_crashes` and `log_crashes_to`.

4. **Start** (`LoadedImage::spawn_main(self: Arc<Self>, dlls)`). A new thread
   named `title-main` with stack size `max(SizeOfStackReserve, 1 MiB)`. On
   it: each title DLL is attached in order (`LoadedImage::attach`: TLS
   callbacks with `DLL_PROCESS_ATTACH`, then `DllMain`; `DllMain` returning
   FALSE is an error), then the executable's TLS callbacks run
   (`run_tls_callbacks`), `threads::attached` records it, and the original
   entry point is called with a null argument. The CLI joins the thread.
   `LoadedImage::detach` and `unload` exist for late loads that fail.

### TLS redirection (`crates/victory-loader/src/tls.rs`)

An executable's code assumes TLS index 0: it reads `gs:[0x58]` (the
thread's TLS array) and dereferences it without reading `_tls_index`.
Mapped as a DLL the image gets another index. `redirect(base, info)` reads
the index the OS wrote (through the TLS directory's `AddressOfIndex`). If it
is 0 nothing happens. Otherwise:

- `near` finds one free page within 1 GiB of the image (64 KiB steps down
  and up), so every `rel32` call reaches it. Images of 1 GiB or more are
  refused.
- `thunks` fills it with 16 thunks, one per register: `mov r, gs:[0x58];
  lea r, [r + index*8]; ret` (flags untouched, like the original load). The
  page is then made read-execute and leaked.
- `patch` scans every executable section byte by byte for `mov r64,
  gs:[0x58]` (`65 REX 8B ModRM 25 58 00 00 00`, REX 48 or 4C, not rsp) and
  rewrites the 9 bytes as `call thunk[r]` plus a 4-byte NOP. The comment
  calls this a byte match, not a decode, checked against one title's full
  disassembly.
- The image's `_tls_index` is set to 0, because CRT code does read it and
  adds it to the array pointer.

DLLs are not redirected: they index TLS properly.

### Threads (`crates/victory-loader/src/threads.rs`)

The prepared copies have no entry point and no TLS callbacks, so Windows
tells them nothing when threads start or end. Our own TLS callback
`ON_THREAD` (linked into `.CRT$XLAV`, which sorts ahead of the usual `XLB`)
runs on every thread of the process. `on_thread` takes a snapshot of the
attached images (title code may map another DLL from inside the notice) and
calls `LoadedImage::notify` on each: TLS callbacks, then `DllMain` unless
`DisableThreadLibraryCalls` made the image quiet. Starts go in load order,
ends in reverse. On a thread Rust's std started (`started_by_rust`: start
routine inside our exe), `OwnTls::save`/`restore` puts our own static TLS
block back afterwards, because std aborts if its per-thread state was
touched before it set the thread up.

State: `IMAGES: Mutex<Vec<Mapped>>` (image, attached, quiet), in attach order.
`disable_thread_calls(base)` is the loader side of `DisableThreadLibraryCalls`
(called from `victory-hle`'s `kernel.rs`); it fails for an image with
static TLS, as Windows does.

Exit: the first `attached` registers `detach_all` with the CRT's `atexit`,
before the title's own exit functions, so it runs after them. `detach_all`
sets `EXITING` (no more thread notices), starts a watchdog thread that ends
the process after `DETACH_DEADLINE` (5 s) without logging or allocating,
ends every other thread (`end_other_threads`, which skips threads whose
start routine is in `ntdll.dll`: the system thread pool), then sends
`DLL_PROCESS_DETACH` to DLLs in reverse initialisation order, then to the
executable.

### Console-only instructions (`console_insn.rs`, `console_insn_decode.rs`)

The console CPU (Zen 2) has instructions a PC may lack. The title raises
`STATUS_ILLEGAL_INSTRUCTION` on them. The vectored handler in `crash.rs`
calls `console_insn::emulate(ctx)` first for every illegal instruction.

- `decode::decode` (in `console_insn_decode.rs`) recognises `clwb`, `clzero`,
  the 12-byte `clzero; add rax,40h; cmp rax,reg; jb` loop (`ClzeroLoop`),
  `rdpru`, `monitorx`/`mwaitx`, `rdpid`, SSE4a `extrq`/`insertq`, and `ud2`
  (never emulated: the title's own trap).
- `emulate` reads up to 16 code bytes (checking they are readable), decodes,
  does the instruction's work on the saved `CONTEXT`, advances `Rip`, and
  returns `CONTINUE_EXECUTION`. It returns 0 for anything it does not handle,
  and the rest of the handler sees it.
- Patching in place so the next pass does not fault: `clwb` becomes NOPs;
  `monitorx`/`mwaitx` (3 bytes) become a 3-byte NOP and `pause; nop`
  (`wait_patch`, `VICTORY_PATCH_WAITS`); a `clzero` loop becomes a jump to a
  stub of plain 8-byte stores (`clzero_stub`, `patch_clzero`,
  `VICTORY_PATCH_CLZERO`). `patch` swaps the bytes with one `cmpxchg16b` on
  the aligned 16 bytes around them, under the `PATCHING` mutex, and refuses
  where they cross 16 bytes. Stubs live in a 64 KiB block from `near(at)`
  (within about 1.75 GiB, `0x10_0000..0x7000_0000` steps), shared through
  `STUBS`.
- Memory checks: `writable_to` and `readable` ask `VirtualQuery`. By default
  each thread remembers the last 4 writable regions and 8 code addresses
  (`WRITABLE`, `CODE`), forgetting regions after 4096 answers
  (`VICTORY_CLZERO_ASK=1` asks every time). A `ClzeroLoop` zeroes region by
  region and sets `rax` and the flags as the last pass would (`cmp_flags`
  from `crash.rs`).
- Diagnostics builds count faults per address (`mod sites`) and log the
  busiest every 5 s.

`crash.rs` keeps an older emulator, `emulate_amd_only` (clzero and its two
loop shapes, `monitorx`/`mwaitx`, `rdpru`, `clwb` and `clwb` loops, SSE4a).
It runs only when `console_insn::emulate` returned 0.

### Look-before-write sites (`crates/victory-loader/src/write_sites.rs`)

Some stores of the title nearly always write what is already there (flags it
sets every frame, moved values). When the target lies in a guarded block of
memory that `victory-gpu` tracks for the card, each such store costs a fault,
a copy and a new guard. `write_sites` rewrites listed instructions into a
jump to a stub of ours that reads the target first and runs the original
instruction only if the bytes would change, with registers and flags left
as the original leaves them.

- **How the places are found.** Not by us ahead of time. With
  `VICTORY_WRITE_SITES=<file>` set, `victory-gpu`'s `titlewrite::learn`
  (`crates/victory-gpu/src/titlewrite.rs`) counts faulting stores by
  instruction address. When a place has left memory unchanged 256 times, and
  changed it at most a quarter as often, and `decode::Store::can_look` says
  the loader can rewrite it, a background thread appends
  `<image file name> <offset hex>` to the file. The image file name is the
  module's file name from `GetModuleFileNameW`, so for the title it is the
  prepared copy, `victory-image-<hash>.dll`. `learn` is only reached from
  `titlewrite::store`, which returns at once unless `VICTORY_STORE_BESIDE=1`
  (read in `titlewrite.rs`).
- **How they are applied.** `apply_asked(name, base, size)` reads the file
  and calls `apply`. The CLI calls it once, with the executable's prepared
  file name, after binding and before any title code runs (`--execute`
  only). For each line naming this image (case-insensitive, offset + 16
  inside the image, each offset once): read 16 bytes, build a stub with
  `stub` (tries `vector_stub`, `move_stub`, `or_stub` in that order), write it
  into a block from `console_insn::near`, fill the original instruction with
  NOPs and put a 5-byte `jmp` at its start, then flush the instruction cache.
  Instructions shorter than 5 bytes, register targets, `rip`-relative and
  `rsp`-based addresses and plain absolute addresses are refused and logged.
- **Stub shapes.** `or_stub`: `or byte [m], imm8`; reads the byte, works out
  the bits that would be set, sets the flags with `or al, imm`, and runs the
  original only if any bit is new (`jecxz`). `move_stub`: `mov [m], reg`
  (`89`, not rcx/rsp) and `mov dword/qword [m], imm32` (`C7 /0`), 4 or 8
  bytes; compares through `not` and `lea` so no flag changes. `vector_stub`:
  VEX `vmovdqu/vmovups/vmovaps` stores of 16 or 32 bytes; compares with
  `vpcmpeqb`/`vpmovmskb` using a spare register saved on the stack; needs
  AVX2 on the host. `write_or_not` emits the common tail.
- The tests run both the original and the rewritten code over many inputs and
  compare memory, registers and flags.

### Crash reporter (`crates/victory-loader/src/crash.rs`)

`report_crashes()` installs a vectored handler (`AddVectoredExceptionHandler`
with First = 0, so it is appended to the list) and an unhandled-exception
filter. `victory-hle`'s `kernel::SetUnhandledExceptionFilter` keeps our
filter in front of the title's and calls `report_unhandled` before the
title's own.

- `handler`: illegal instruction goes to `console_insn::emulate`, then
  `emulate_amd_only`. Otherwise, for the `FATAL` codes (access violation,
  illegal or privileged instruction, divide by zero, stack overflow,
  breakpoint) it prints a "first-chance exception" report, and always
  returns `EXCEPTION_CONTINUE_SEARCH`.
- `report`: code, `locate(address)` as `module+rva`, the detail (for an access
  violation, read/write/execute and `memory_at`: free, reserved, guard page or
  the protection and module; for an MSVC C++ throw, the thrown type and an
  integer value or `std::exception` message via `thrown`; for illegal
  instructions, the bytes via `instruction_bytes`, never past the page),
  general registers, and up to `MAX_FRAMES` (40) frames from `unwind`.
- `unwind_into` walks with `RtlLookupFunctionEntry` and `RtlVirtualUnwind`
  over a 16-byte aligned copy of the context (`Aligned16`). Code without
  unwind data is assumed to have its return address at `rsp`. It stops at
  the stack bounds and does not allocate past the vector's capacity, so it can
  run while another thread is suspended holding the heap lock.
- Crash log: `log_crashes_to(file)` sets the file and adds a panic hook. Only
  unhandled reports, panics and `log_crash` texts are appended (with Unix time
  and process id). `note_title_output` keeps the title's last 8 lines of
  debug output (fed from `kernel::OutputDebugString*`), appended to each
  logged report.
- `save_on_crash` (diagnostics) writes named memory ranges to files on a
  report; the CLI fills it from `VICTORY_CRASH_SAVE`.

### Stack dumps and profiler (diagnostics builds)

`stacks.rs`: `dump_stacks_on_schedule` reads `VICTORY_STACKS` and prints
every thread's stack at those times. `frames_of` suspends a thread, reads
its TEB stack bounds and context, unwinds with `crash::unwind_into` into a
buffer allocated beforehand, then resumes it. `profile.rs`:
`profile_on_schedule(phase)` samples the 28 busiest threads (chosen anew each
second by CPU time) every few milliseconds between two times and writes
counted stacks as `module+rva`. `phase` is `victory_hle::frame_phase`.

### What is per build

Nothing in this crate holds offsets or patterns of a title build.

- The build identity is `PreparedImage::original_hash` (BLAKE3 of the
  original executable). The CLI looks it up with `victory_game::identify`
  (`crates/victory-game/src/lib.rs`), which searches `KNOWN_BUILDS`. That
  list is empty today, so every build prints as "unknown". Nothing else
  depends on the result.
- The hash names the prepared copy (`victory-image-<hash16>.dll`). That file
  name is what crash reports, stack dumps and profiles print as `module+rva`,
  and what the write-sites file is keyed by, so offsets learned for one build
  are ignored by another.
- TLS redirection, console-instruction emulation and write-site stubs work
  from the bytes found at load or at the fault.

## Files

| Path | What it does | Key items |
|------|--------------|-----------|
| `crates/victory-loader/src/lib.rs` | Crate root: the five steps in its docs, module gating, re-exports. | `DLL_PROCESS_ATTACH`, `DLL_PROCESS_DETACH` |
| `crates/victory-loader/src/prepare.rs` | Patches a copy of an image so Windows maps it without running it or binding it. Portable. | `prepare`, `PreparedImage`, `PreparedImage::file_name`, `dir64_relocations` |
| `crates/victory-loader/src/publish.rs` | Writes a derived file into a shared folder through a per-writer staging file and an atomic rename. | `write_if_changed` |
| `crates/victory-loader/src/resolve.rs` | The resolver interface and its result types. | `ImportResolver`, `Resolution`, `Provenance` |
| `crates/victory-loader/src/windows.rs` | Maps prepared images, maps title DLLs recursively, binds IAT slots, attaches and detaches images, starts the title thread. | `load`, `load_title_dlls`, `load_title_dll`, `WithTitleDlls`, `LoadedImage::{bind_imports, attach, detach, notify, spawn_main, unload, export_symbol}`, `BindReport` |
| `crates/victory-loader/src/thunks.rs` | Generated stubs for imports nobody implements. | `UnimplementedPolicy`, `ImportSite`, `ThunkBlock`, `unimplemented_import`, `unimplemented_call_count` |
| `crates/victory-loader/src/tls.rs` | Redirects an executable's TLS-array loads to its real slot when mapped as a DLL. | `redirect`, `patch`, `thunks`, `near` |
| `crates/victory-loader/src/threads.rs` | Our TLS callback that passes thread start and end to attached title images; process-exit detach with a watchdog. | `ON_THREAD`, `on_thread`, `mapped`, `attached`, `detached`, `detach_all`, `end_other_threads`, `disable_thread_calls`, `OwnTls` |
| `crates/victory-loader/src/console_insn.rs` | Emulates console-only CPU instructions at their fault and patches the hottest ones in place. Ported from XCL (see Origins). | `emulate`, `patch`, `patch_clzero`, `clzero_stub`, `wait_patch`, `near`, `jump`, `STUBS_LEN`, `writable_to` |
| `crates/victory-loader/src/console_insn_decode.rs` | Decoder for those instructions (included by `console_insn.rs` with `#[path]`). | `ConsoleOnly`, `decode`, `extrq`, `insertq` |
| `crates/victory-loader/src/write_sites.rs` | Rewrites listed stores of the title to look before they write. | `apply`, `apply_asked`, `stub`, `or_stub`, `move_stub`, `vector_stub`, `write_or_not` |
| `crates/victory-loader/src/crash.rs` | Vectored handler and unhandled filter: emulation hand-off, reports as `module+rva` with unwound frames, crash log, older AMD-only emulator. | `report_crashes`, `report_unhandled`, `log_crashes_to`, `log_crash`, `note_title_output`, `locate`, `unwind_into`, `emulate_amd_only`, `cmp_flags`, `CMP_FLAGS`, `save_on_crash` |
| `crates/victory-loader/src/stacks.rs` | Diagnostics: every thread's stack at scheduled times. | `dump_stacks_on_schedule`, `dump_stacks`, `frames_of`, `other_threads` |
| `crates/victory-loader/src/profile.rs` | Diagnostics: a sampling profiler of the busiest threads. | `profile_on_schedule`, `asked`, `choose`, `BUSIEST` |
| `crates/victory-loader/tests/load_probe.rs` | End-to-end test: builds the hle-probe fixture, prepares, loads, binds with a test resolver, calls its exports, checks thread notices, detach and late DLL mapping. | `build_probe`, `TestResolver`, `loads_binds_and_calls_probe`, `maps_a_title_dll_the_title_loads_itself` |
| `crates/victory-loader/tests/title_dlls.rs` | A title DLL that is not a PE is named in the error. | `a_title_dll_that_does_not_prepare_is_named` |
| `tests/fixtures/hle-probe/src/lib.rs` | A stand-in "title" DLL written by us: imports from DLLs a PC does not have (`raw-dylib`), a delay-loaded one, a TLS callback and a `DllMain` that count thread starts. | `probe_xgameruntime_init`, `probe_tick`, `probe_pid`, `probe_title`, `probe_delayed`, `probe_missing`, `probe_thread_starts`, `probe_detaches` |
| `tests/fixtures/hle-probe/build.rs` | Makes `victory-probe-delayed.dll` delay-loaded (`/DELAYLOAD`, `delayimp`). | `main` |
| `tests/fixtures/hle-probe/Cargo.toml` | Not a workspace member (`exclude = ["tests/fixtures"]` in the root manifest); a `cdylib` with `panic = "abort"`. The test builds it into its own target directory under `CARGO_TARGET_TMPDIR`. | |

## Switches

| Name | Default | What it changes | Read in |
|------|---------|-----------------|---------|
| `VICTORY_WRITE_SITES=<file>` | unset: off | The list of look-before-write places to apply at load. The same variable makes `victory-gpu` append places to it. | `crates/victory-loader/src/write_sites.rs` (`apply_asked`); also `crates/victory-gpu/src/titlewrite.rs` |
| `VICTORY_CLZERO_ASK=1` | unset: regions and code bytes are remembered per thread | Ask `VirtualQuery` at every emulated instruction. | `crates/victory-loader/src/console_insn.rs` (`remembers`) |
| `VICTORY_PATCH_WAITS=0` | on | Emulate `monitorx`/`mwaitx` every time instead of replacing them at the first fault. | `crates/victory-loader/src/console_insn.rs` (`patches_waits`) |
| `VICTORY_PATCH_CLZERO=0` | on | Emulate `clzero` loops every time instead of replacing them with plain stores. | `crates/victory-loader/src/console_insn.rs` (`patches_clzero`) |
| `VICTORY_STACKS=<s>[,<s>...]` | unset | Diagnostics: print every thread's stack at those seconds. | `crates/victory-loader/src/stacks.rs` |
| `VICTORY_PROFILE=<from>,<to>,<ms>,<file>[;...]` | unset | Diagnostics: sample the busiest threads' stacks and write them to `<file>`. | `crates/victory-loader/src/profile.rs` |
| `VICTORY_CRASH_SAVE=<rva>:<len>,...` | unset | Diagnostics: title ranges written to files on a crash. Read by the CLI, passed to `save_on_crash`. | `apps/victory/src/run.rs` |
| `VICTORY_TRAP_LOG`, `VICTORY_EXIT_CHILD` | unset | Test-only: child processes of `thunks.rs` and `threads.rs` tests. | `thunks.rs`, `threads.rs` |

## Traps

- **Order in `run`.** Bind title DLLs and the executable, then apply write
  sites, then `report_crashes`, then `spawn_main`. Write-site patching and
  TLS redirection both assume no title code has run yet.
- **Import directory.** `prepare` points it at the empty terminator rather
  than zeroing it (static TLS on Windows 10, per the comment). Test
  `keeps_iat_slots_when_import_size_excludes_the_terminator` covers a linker
  whose import size leaves out the terminator.
- **TLS callback relocation.** Clearing `AddressOfCallBacks` is not enough if
  the image is rebased; its base relocation is neutralised too.
- **`PreparedImage::info` describes the original**, not the patched bytes.
  `LoadedImage::info` is a clone of it.
- **Leaks are deliberate.** Import stubs (`ThunkBlock::seal`), `ImportSite`s,
  TLS thunks and instruction stubs are never freed: title code may hold their
  addresses for the life of the process.
- **TLS redirect is a byte match.** It finds `65 REX 8B /r 25 58 00 00 00`
  anywhere in executable sections. The comment records that every hit in the
  title was an instruction start; a new title could break that.
- **Rel32 reach.** TLS thunks need a page within 1 GiB; instruction and
  write-site stubs use `console_insn::near` (up to about 1.75 GiB away).
  `write_sites::apply` allocates its own 64 KiB block per call and stops
  when it is full.
- **`write_sites` needs at least 5 bytes** for its jump and fills the rest of
  the instruction with NOPs. No thread may be inside the patched range, which
  holds because it runs before title code. `console_insn::patch_clzero`
  relies instead on no thread ever being inside the loop's other
  instructions, since the emulation does the whole loop in one fault.
- **Thread notices.** `on_thread` takes a snapshot of `IMAGES` before
  calling title code (title code may map a DLL from there). Do not hold the
  `IMAGES` lock across `notify`. `OwnTls` restore is what keeps Rust's std
  from aborting on threads it starts; test `loads_binds_and_calls_probe`
  checks it with a named worker thread.
- **Exit.** `detach_all` must not log after the watchdog's deadline: an
  ended thread may hold the heap or log lock. Threads starting in `ntdll`
  are not terminated (the system pool fails fast on `TerminateThread`).
- **Two emulators.** `console_insn::emulate` runs first. When it returns 0
  (for example `clzero` on memory it finds not writable, which its comment
  says should fault like the real instruction), `crash::emulate_amd_only`
  still sees the same opcode and its `clzero` writes the line without a
  check. Whether that path is reached at run time is not verified.
- **`emulate` shortcut.** A remembered writable region that was freed or
  protected since may be written for up to 4096 answers. The comment says
  the write then faults inside the handler, which a view guard opens as any
  write, and is a crash otherwise.
- **Crash handler is appended** to the vectored list (First = 0). Other
  vectored handlers registered with First = 1 run before it.
- **`unwind_into` must not allocate** beyond `MAX_FRAMES`: `stacks.rs` calls
  it while the target thread is suspended.
- **`load`'s folder is checked** to be outside the repository; the prepared
  copy is derived game data (see `docs/asset-policy.md`).

## Origins

- Loader, binding, thunks, prepare, crash reporter, thread notices and TLS
  redirection: VIctory by Marvis (`f4504f6`, `754619f`, `4b25269`,
  `2849477`, and later commits). The commit log also names, for crash.rs and
  the emulation: ismailchaka1 (`934e67f`, AMD-only instructions on Intel),
  raffclar (`0c91c03`, `f00caec`, `d4ed2de` exit watchdog), Henrique Bueno
  (`71cfe84`, `c4e915f`, clwb), Duff (`babe2db`, `3390e47`, crash report
  details), Abdellah K. (`b5c6678`, aligned unwind context), Maledetto
  (`1466eb6`, clzero loops, rdpru), 给伊莉雅补魔 (`6aa6fc1`), BastienRdz
  (`ea78bb9`), Tex (`32676e3`). `publish.rs`: quaternion (`7a4f001`).
- `console_insn.rs` and `console_insn_decode.rs`: added in fxceless's
  `578d459` ("...: this fork's changes on top of VIctory main").
  The file's own header says it is ported from XCL
  (github.com/ivall/xcl, GPL-2.0-only): its `insn.rs` decoder "unchanged" as
  `console_insn_decode.rs`, and `eh.rs` `emulate_console_only`. The commit
  message says XCL "was used as a reference". This fork's later changes:
  `daeeb28`, `2132366`, `13ffc28`.
- `write_sites.rs` (`83a1bcf`, `e8d3d91`) and `profile.rs` (`6ff1e14`):
  this fork.

## Where to start for common changes

- The title stops at a missing import: the stub names it (log and crash log).
  Implement it in `victory-hle` (see `docs/agents/victory-hle.md`), or run
  with `--unimplemented zero` to push on.
- A crash in title code: read the crash log (beside the title's `temp`
  folder, `crashes.log`), then `locate` output `victory-image-<hash>.dll+rva`.
- A new console-only instruction: add a `ConsoleOnly` variant and its bytes
  in `console_insn_decode.rs` (`decode`), then its effect in
  `console_insn.rs` (`emulate`). The tests feed `emulate` a fake `CONTEXT`, so
  they run on any CPU.
- A new kind of look-before-write store: a `*_stub` in `write_sites.rs`
  added to `stub`, and the matching `can_look` case in
  `crates/victory-gpu/src/titlewrite/decode.rs`, or `victory-gpu` will never
  list it.
- Title DLL handling: `windows.rs` (`map_dll`, `WithTitleDlls`) and the
  `wanted` filter in `apps/victory/src/run.rs` (`title_dll`).
- Thread or exit behaviour of title images: `threads.rs`.
