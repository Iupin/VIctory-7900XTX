# victory-hle

## Purpose

Our implementation of the console API surface a title imports, and the
policy for what passes through to the host's own Windows DLLs. The loader
asks `HleResolver` for every `dll!symbol` while binding
(`crates/victory-loader`, see `docs/agents/victory-loader.md`); title code
then calls straight into the `extern "system"` functions here. GPU work goes
on to `victory-gpu` (`d3d12x.rs` hands device creation to it), texture
layouts and audio codecs come from `victory-formats`. The resolution order
and the `LoadLibrary`/`GetProcAddress`/`QueryApiImpl` rules are stated in
[docs/architecture.md, "Import resolution"](../architecture.md#import-resolution);
this map shows where they live in code.

## How it works

### Resolving an import

`crates/victory-hle/src/lib.rs`, `HleResolver::resolve(dll, symbol)`:

1. `registry::lookup(dll, &symbol.to_string())`: an ordinal is looked up as
   `#n`. If found, `Provenance::Hle`.
2. Else `HostPolicy::resolve(dll, symbol)`: if the DLL matches an allow
   pattern, the host's export from System32, `Provenance::Host`.
3. Else `Resolution::Missing`; the loader then adds title-DLL exports
   (`WithTitleDlls`) and stubs.

`HleResolver::classify` is the same without binding (used by `victory
census` and the tests). `--strict` in the CLI uses `HostPolicy::none()`.

### How a module declares what it implements

`crates/victory-hle/src/registry.rs`:

- `HleExport { name, address }`: one function under the exact imported
  name (`"#n"` for an ordinal).
- `HleModule { name, dlls, exports }`: `dlls` are lower-case DLL names; a
  trailing `*` matches a prefix (versioned API sets such as
  `api-ms-win-core-file-*`). `HleModule::serves(dll)` compares
  case-insensitively.
- `hle_export!(f)` builds an `HleExport` whose name is the Rust identifier
  of `f`. For names that are not identifiers (`#1`, C++ decorated names) a
  module writes `HleExport { name: "...", address: f as *const () as usize }`
  by hand (`gameinput.rs`, `files.rs`, `xmem.rs`).
- `REGISTRY` is a `LazyLock` over `modules::all()` (`modules/mod.rs`), which
  calls each file's `pub(crate) fn module() -> HleModule`, some only on
  Windows. `registry()` exposes it; `lookup` takes the first module, in
  `all()` order, that serves the DLL and has the name.

Several modules serve the same DLL: `kernel32.dll` is served by `kernel`,
`files` and `xmem`; each lists different functions.

### `HostPolicy` (`crates/victory-hle/src/host.rs`)

`HostPolicy { allow: Vec<String> }`, patterns as above. `Default` allows the
Windows API sets and runtimes the console shares with PC (`api-ms-win-core-*`,
`api-ms-win-crt-*`, `kernel32`, `ntdll`, `ucrtbase`, `vcruntime140*`,
`msvcp140*`), networking and crypto DLLs (refused off-machine by
`modules::net`), window and shell API sets, `ole32`, `advapi32`, `dbghelp`,
`xaudio2_9.dll`, `mfplat.dll`, `mfreadwrite.dll`, and a few more (read the
list in `Default::default`). `HostPolicy::none()` allows nothing. Console-only
DLLs (`xmem.dll`, `xgameruntime.dll`, ...) are never in it.

`HostPolicy::resolve` gets the address through `imp::resolve`, which loads
the DLL with `LoadLibraryExA(..., LOAD_LIBRARY_SEARCH_SYSTEM32)` and caches
the handle in `MODULES`. Two wrappers can replace the host address:
`victory_gpu::titlewrite::import` swaps `memcpy`, `memmove` and `memset` for
write-tracking copies when `VICTORY_COPY_BESIDE=1` (read in
`crates/victory-gpu/src/titlewrite.rs`), and in diagnostics builds
`victory_gpu::timeline::import` wraps what threads wait with.
`host::function(dll, name)` gives the host's own function, for overrides that
call through (the `host!` macro in `files.rs` caches one per call site).

### Dynamic loading (`modules/kernel.rs`)

`LoadLibraryEx{A,W}` and `LoadLibrary{A,W}` call `find_module(name, load)`:

1. If the load runs code (`runs_code(flags)`), `runtime::load_title_dll`
   (the loader closure the CLI set) maps a title DLL not mapped yet. An error
   returns null with `ERROR_MOD_NOT_FOUND`; it does not fall back to the host,
   which would load the unpatched DLL.
2. A mapped title image by file name (`runtime::image_named`).
3. If `HostPolicy::default()` does not allow the name and a registry module
   serves it: the pseudo handle `handle(m)`, the address of that `HleModule`.
4. Allowed host DLLs go to the host. Anything else is refused (logged once,
   null, `ERROR_MOD_NOT_FOUND`).

`GetProcAddress(module, name)`: a null module means the title's exe. On a
pseudo handle it looks up the module's exports by name; ordinals return null.
On a real host module, `host_override` first checks the registry for that
file and name, so a lookup by name gets the same override binding would
(the network guard, path rewriting). `GetModuleHandle{A,W}(NULL)` and
`GetModuleFileName{A,W}` answer with the title's image and its original
path (`runtime::main_image`, `runtime::image_path`). `FreeLibrary` on a
pseudo or title module does nothing. `OutputDebugString*` goes to our log
and to `victory_loader::note_title_output`. `SetUnhandledExceptionFilter`
puts `report_unhandled` in front of the title's filter. `exit`, `abort`,
`terminate` and `TerminateProcess` log the caller. `SetThreadPriority` lifts
the title's idle and lowest priorities to below normal
(`VICTORY_TITLE_PRIORITIES=0`: as asked). `__p___argc`/`__p___argv` give the
title its own argument list (`runtime::title_args`).

### Runtime objects and `QueryApiImpl` (`modules/xgameruntime.rs`)

A stub linked into the title loads `xgameruntime.dll` with `LoadLibraryExA`
(a pseudo handle), finds the `*ApiImpl` functions with `GetProcAddress`, and
initialises. Every runtime call then does `QueryApiImpl(provider, iid,
&out)`, calls one method of the object and releases it.

- `Object { vtable, area }`: `#[repr(C)]`, first word the method table, as
  title code expects of a COM-style object. Objects are `static`; `add_ref`
  serves `AddRef` and `Release` and returns 1.
- `Methods<N>`: the table by slot. Slot 0 is `QueryInterface`, 1 and 2
  `AddRef`/`Release`, then the area's methods from byte offset 0x18.
  `missing::<SLOT>` fills slots nobody needed yet: it logs the area and the
  byte offset and returns `E_NOTIMPL`.
- `Interface = (provider Guid, interface Guid, &'static Object)`. Each area
  file exports `pub const INTERFACES`; `AREAS` in `xgameruntime.rs` lists
  them all. `QueryApiImpl` searches `AREAS` for the exact pair. A pair not
  found returns `NOT_SUPPORTED` and is logged once; `out` is left untouched.
  An area whose method table changed has a new interface id, so an object
  answers only the ids whose slots we know (`storage.rs` answers two).
- Helpers for method bodies: `put`, `put_string`, `text`, `token`, the
  `HRESULT` constants (more in `xasync.rs`: `E_INVALIDARG`, `E_PENDING`,
  `E_ABORT`, `E_NOT_SUFFICIENT_BUFFER`, ...). Async methods take an
  `xasync::AsyncBlock` and complete through `xasync`.

Areas (offline retail console, one signed-in user): `xasync`, `xuser`,
`xsystem`, `xgame`, `storage`, `xnetworking`, `xpackage`, `display`,
`xgamesave`, `xstore`, `xgameui`, `speech`.

Other console DLLs (`xapu.dll`, `chad.dll`, `xcrdapi.dll`, `d3d12_xs.dll`,
`xg_xs.dll`, ...) use their own COM-style objects built the same way
(`Object`, `Methods`, `missing`) and handed out by their own factory
functions (`DStorageGetFactory`, `XGCreateTextureComputer`, `AcpHalCreate`).

### Process-wide state (`crates/victory-hle/src/runtime.rs`)

Set by the CLI before title code runs: `set_main_image` (also starts
`image_watch`), `set_image_path` per title image, `set_title(Title)`
(from `Title::from_manifest` of `MicrosoftGame.config`: id, store id,
storage quota; the CLI fills `storage`), `set_title_args`,
`set_title_dll_loader`. `use_console_timers` sets 1 ms timers, opts out of
power throttling and raises the process to above normal priority.

### Virtual drives and paths (`modules/files.rs`, `modules/storage.rs`)

`DRIVES: Mutex<Vec<(letter, host root UTF-16)>>`. `mount_at(letter, root)`
fixes a letter: the CLI mounts the package folder as `G:` and a temp folder
under the data directory as `T:` (a host drive of that letter is hidden).
`mount(root)` picks the highest letter from D to Z that neither the host
nor another mount uses, or returns the letter the folder already has.
Persistent local storage is mounted this way on first use
(`storage::folder`), and the title gets `X:\` as its path.

Every path-taking import of kernel32, the file API sets, the CRT and
`msvcp140` (`std::_Fiopen`) is exported from `files.rs`. Each wrapper calls
`translate(path)`: a path on a mounted letter (`X:\..`, `X:/..`, `X:`,
`\\?\X:\..`) is rewritten onto the host root; host paths of 248 characters
or more get the `\\?\` form with `.`/`..` resolved by `verbatim`. Any other
path is passed through unchanged. Narrow paths are decoded with the ANSI code
page and go to the host's wide function. `ReadFile` and `fread` also call
`victory_gpu::kernel_writing` around the host call, which opens guarded
blocks of the buffer and keeps them open until the call returns (the kernel's
writes fail on a guard instead of faulting into the handler). `media.rs`
gives the same rewriting to Media Foundation's source resolver.

Game saves (`xgamesave.rs`) are folders under `Title::saves()`, beside
persistent storage, with all-or-nothing updates by directory rename.

### Audio

Three console coprocessors, each its own DLL, and the host's XAudio2:

- `AcpHal.dll` (`modules/audio.rs`): the APU heap (host memory with 32-bit
  device addresses, `ApuAlloc`, `ApuMapApuAddress`, ...), SHAPE context
  arrays, and the audio control processor object (`AcpHalCreate`) whose
  commands complete at once with the acknowledgements the hardware sends.
  Enabled XMA contexts are decoded with `victory_formats::xma` by one thread,
  "victory XMA decoder" (`wake_decoder`, every 2 ms), into their output rings.
  SHAPE's other units (rate conversion, filters, mixing) are not emulated;
  voices that need them are silent. Mixed output leaves through the host's
  WASAPI, reached by the title through COM.
- `xapu.dll` (`modules/apu.rs`): Opus decoding with `opus_pure`. Commands are
  decoded when queued, so each result is ready at once and in order.
- `chad.dll` (`modules/dsp.rs`): DSP streams (reverb convolution and other
  block transforms). Commands complete when submitted; outputs are zeros
  because the hardware's impulse-response layout is not known.
- `xaudio2_9.dll` passes through to the host (`HostPolicy`).
- `modules/audio/capture.rs` (diagnostics): records what the process plays
  with process loopback, from a child copy of the executable.

### Threads started here

- `dstorage {i}`: one DirectStorage worker per available CPU (`scheduler`),
  shared by every queue.
- `mapping {i}`: `xmem.rs` page-mapping helpers (`side_by_side`: 8 by default).
- `victory XMA decoder` (`audio.rs`), `victory-image-watch` (`image_watch.rs`).
- Unnamed threads in `xasync.rs` (task-queue thread-pool ports, timed
  submissions), `gameinput.rs` (the poll thread that turns host state into
  readings and runs device callbacks), `lifecycle.rs` (suspend routines on
  window close) and others. Our threads that run title callbacks are Rust
  threads; title images get their notices from `victory-loader::threads`.

### Adding a new import implementation (recipe)

1. Find the DLL. If `modules/` already has a file whose `module()` serves it
   (see the table below), add to that file. Otherwise create
   `crates/victory-hle/src/modules/<area>.rs`, add `pub mod <area>;` to
   `modules/mod.rs` (inside `#[cfg(windows)]` if it uses `windows-sys`) and
   push `<area>::module()` in `all()`.
2. Write the function with the exact import name and the console ABI:
   ```rust
   #![allow(non_snake_case)]
   pub extern "system" fn XThingGetValue(handle: *mut c_void, out: *mut u32) -> HRESULT {
       super::xgameruntime::put(out, 0)
   }
   ```
   Use `extern "C"` for CRT functions (`files.rs`). Pointers from the title
   are trusted as the real API would trust them
   (`#![allow(clippy::not_unsafe_ptr_arg_deref)]` at the top of such files).
3. List it in the module's `exports`: `hle_export!(XThingGetValue)`, or
   `HleExport { name: "#7", address: f as *const () as usize }` for an
   ordinal or a decorated name.
4. If the DLL is one `HostPolicy` passes through, nothing else is needed:
   the registry wins over the host for the names you list, both at binding
   and through `GetProcAddress`. To call the host's own function from your
   override use `crate::host::function(dll, name)` (or `files.rs`'s `host!`).
5. For a runtime area reached through `QueryApiImpl`: define the provider and
   interface `Guid`s, a `static VTABLE: Methods<N>` (`missing::<0>`,
   `add_ref`, `add_ref`, then your methods, `missing::<k>` for gaps),
   `pub static OBJECT: Object = Object::new("<area>", &VTABLE)` and
   `pub const INTERFACES: &[Interface]`, then add `<area>::INTERFACES` to
   `AREAS` in `xgameruntime.rs`. `speech.rs` is the smallest example.
6. Test: `HleResolver::new(HostPolicy::none()).classify(dll, &name)` returns
   `Some(Provenance::Hle)` (`lib.rs` tests); call object methods with
   `xgameruntime::method::<F>(&OBJECT, slot)`.
7. `victory census` over the title lists what is still missing, most used
   first (`--missing`).

## Files

| Path | What it does | Key items |
|------|--------------|-----------|
| `crates/victory-hle/src/lib.rs` | Crate root: `HleResolver` (registry, then host), re-exports of `victory-gpu` items the CLI needs. | `HleResolver::{resolve, classify}`, `HostPolicy`, `registry`, `frame_phase`, `console`, `known_pipelines` |
| `crates/victory-hle/src/registry.rs` | The table of everything implemented and its lookup. | `HleExport`, `HleModule::serves`, `hle_export!`, `registry`, `lookup` |
| `crates/victory-hle/src/host.rs` | Which DLLs pass through to the host, and loading them from System32 only. | `HostPolicy::{default, none, allows, resolve}`, `function`, `imp::resolve`, `imp::module` |
| `crates/victory-hle/src/runtime.rs` | Process-wide state: main image, image paths, title args, title DLL loader, manifest, console timers and priority. | `set_main_image`, `image_path`, `image_named`, `set_title_dll_loader`, `load_title_dll`, `Title::{from_manifest, saves}`, `use_console_timers` |
| `crates/victory-hle/src/image_watch.rs` | Diagnostics: logs named words of the title image once a second when they change. | `start`, `words` |
| `crates/victory-hle/src/modules/mod.rs` | Declares the module files and builds the registry list. | `all` |
| `crates/victory-hle/src/modules/kernel.rs` | kernel32/kernelbase overrides: module handles and names, `LoadLibrary*`/`GetProcAddress`/`FreeLibrary` with pseudo handles, debug output, exit and abort logging, exception filter, thread priorities, `argc`/`argv`. | `find_module`, `GetProcAddress`, `host_override`, `LoadLibraryExA`, `SetUnhandledExceptionFilter`, `title_priority`, `callers` |
| `crates/victory-hle/src/modules/files.rs` | Virtual drives; every path-taking file import of kernel32, the CRT and `msvcp140`; guarded-buffer handling for reads. | `mount`, `mount_at`, `translate`, `verbatim`, `host!`, `ReadFile`, `fread`, `CreateFileW` |
| `crates/victory-hle/src/modules/xmem.rs` | `xmem.dll`: console memory from host memory; physical 64 KiB pages as pagefile-backed sections mapped into placeholders; `VirtualFree` for title memory; tells `victory-gpu` when DirectStorage wrote pages. | `XMemAlloc`, `XMemAllocatePhysicalPages`, `XMemMapPhysicalPages`, `Section`, `page_window`, `pages_written`, `map_alias`, `side_by_side`, `lift_thread` |
| `crates/victory-hle/src/modules/dstorage.rs` | `xcrdapi.dll`: console DirectStorage; factory, files, queues, status arrays; one worker pool with weighted round robin; zlib and BCPack; writes to addresses or physical pages. | `DStorageGetFactory`, `Scheduler`, `work`, `queue`, `execute`, `into_pages`, `Io`, `stream_log` |
| `crates/victory-hle/src/modules/d3d12x.rs` | `d3d12_xs.dll`: device creation (handed to `victory_gpu`, with `xmem` as allocator and page mapper), root signature serialisation, shader cache folders. | `create_device`, `use_shader_cache`, `precompile`, `warm_shader`, `host_gpu_failed` |
| `crates/victory-hle/src/modules/xg.rs` | `xg_xs.dll`: texture layout computer, served from `victory_formats::texture`. | `XGCreateTextureComputer`, `XGSuggestSwizzleMode` |
| `crates/victory-hle/src/modules/gameinput.rs` | `gameinput.dll` (ordinal 1): XInput pads and the keyboard as devices; a stand-in pad; a poll thread. | `GameInputCreate`, `one_pad`, `FALLBACK_BUTTONS`, `host` |
| `crates/victory-hle/src/modules/audio.rs` | `AcpHal.dll`: APU heap, SHAPE contexts, audio control processor queues, XMA decoding thread. | `ApuAlloc`, `AcpHalCreate`, `Xma`, `decode`, `wake_decoder`, `xma_command` |
| `crates/victory-hle/src/modules/audio/capture.rs` | Diagnostics: records the process's own audio to a WAV file from a child process. | `start` |
| `crates/victory-hle/src/modules/apu.rs` | `xapu.dll`: Opus packets in, float PCM out, results in command order. | `XApuConnect`, `XApuEnqueueCommand`, `XApuDequeueResult` |
| `crates/victory-hle/src/modules/dsp.rs` | `chad.dll`: DSP streams; commands complete at once; outputs are silence. | `XDspConnectWithMaximumStreamLimit`, `XDspSubmitCommand` |
| `crates/victory-hle/src/modules/lifecycle.rs` | App-state and constrained-change notifications, `CreateWindowEx*` (fits the window to the screen), closing the window as the console closes a title, relaunch hand-over. | `RegisterAppStateChangeNotification`, `CreateWindowExW`, `fit`, `wait_for_previous` |
| `crates/victory-hle/src/modules/media.rs` | `mfplat.dll`: a source resolver proxy that opens local files only, with virtual-drive paths rewritten. | `MFCreateSourceResolver` |
| `crates/victory-hle/src/modules/net.rs` | `ws2_32.dll` and `winhttp.dll` calls that leave the machine: loopback only, lookups fail, proxies off. | `connect`, `getaddrinfo`, `WinHttpConnect` |
| `crates/victory-hle/src/modules/rpcrt4.rs` | `rpcrt4.dll`: the GUID calls only; the DLL itself is not passed through. | `UuidCreate`, `UuidToStringA`, `RpcStringFreeA` |
| `crates/victory-hle/src/modules/xgameruntime.rs` | `xgameruntime.dll`: initialisation, `QueryApiImpl` over all areas, the object model and helpers. | `QueryApiImpl`, `AREAS`, `Object`, `Methods`, `Interface`, `missing`, `add_ref`, `put`, `put_string`, `Guid` |
| `crates/victory-hle/src/modules/xasync.rs` | Runtime area: XTaskQueue and XAsync (work and completion ports, dispatch modes, async blocks). | `INTERFACES`, `AsyncBlock`, `E_PENDING`, `E_NOTIMPL` |
| `crates/victory-hle/src/modules/xuser.rs` | Runtime area: one offline local user, signed in, every privilege. | `INTERFACES` |
| `crates/victory-hle/src/modules/xsystem.rs` | Runtime area: console identity and sandbox; also the registry calls analytics use for the device type (`advapi32`, registry API sets). | `INTERFACES`, `RegOpenKeyExW`, `RegQueryValueExW`, `module` |
| `crates/victory-hle/src/modules/xgame.rs` | Runtime area: title id from the manifest, which areas exist, invites and events (none offline). | `INTERFACES` |
| `crates/victory-hle/src/modules/storage.rs` | Runtime area: persistent local storage as a virtual drive with the manifest's quota. | `INTERFACES`, `folder`, `IID_2026` |
| `crates/victory-hle/src/modules/xgamesave.rs` | Runtime area: game saves on the host under the data directory, versioned containers committed by rename. | `INTERFACES` |
| `crates/victory-hle/src/modules/xnetworking.rs` | Runtime area: network stack up but reaching nothing. | `INTERFACES` |
| `crates/victory-hle/src/modules/xpackage.rs` | Runtime area: the title's own package, complete; no other package. | `INTERFACES` |
| `crates/victory-hle/src/modules/xstore.rs` | Runtime area: local active licence, no add-ons, store-service calls fail with the store network error. | `INTERFACES` |
| `crates/victory-hle/src/modules/xgameui.rs` | Runtime area: system dialogs; text entry opens a small host dialog on its own thread. | `INTERFACES` |
| `crates/victory-hle/src/modules/display.rs` | Runtime area: HDR mode, screen-timeout deferrals, launching URIs, screenshots. | `INTERFACES` |
| `crates/victory-hle/src/modules/speech.rs` | Runtime area: speech synthesis returning empty WAVE streams. Smallest area file. | `INTERFACES`, `VTABLE`, `SILENCE` |
| `crates/victory-hle/tests/no_dll_planting.rs` | Host pass-through never loads a same-named DLL from the current folder. | `ignores_dlls_in_current_folder` |

## Switches

| Name | Default | What it changes | Read in |
|------|---------|-----------------|---------|
| `VICTORY_IMAGE_WATCH=name=off:bytes,...` | unset | Diagnostics: log words of the title image when they change. | `crates/victory-hle/src/image_watch.rs` |
| `VICTORY_EFFICIENCY_MODE=1` | unset: the process opts out of power throttling | Leave the system its efficiency mode. | `crates/victory-hle/src/runtime.rs` |
| `VICTORY_PROCESS_PRIORITY=normal` (or `0`) | above normal | Keep the priority class the process started with. | `crates/victory-hle/src/runtime.rs` |
| `VICTORY_TITLE_PRIORITIES=0` | idle..lowest lifted to below normal | Thread priorities as the title sets them. | `crates/victory-hle/src/modules/kernel.rs` |
| `VICTORY_DSTORAGE_IO=one\|handles\|overlapped\|direct` | `handles` | How DirectStorage workers read files. | `crates/victory-hle/src/modules/dstorage.rs` |
| `VICTORY_DSTORAGE_HALF=1` | off | A queue submits itself once half full. | `dstorage.rs` |
| `VICTORY_DSTORAGE_RETRIES=<n>` | 12 | Retries of a page write the host refuses for a moment. | `dstorage.rs` |
| `VICTORY_DSTORAGE_LOG=<file>` | unset | Streaming log, a line per request. | `dstorage.rs` (`stream_log`) |
| `VICTORY_DSTORAGE_TRACE=0\|1\|2` | 0 | Streaming trace of batches and suspicious requests. | `dstorage.rs` |
| `VICTORY_WALLS_TRACE=0\|1\|2` | 0 | Failed (1) or all (2) read requests with their files. | `dstorage.rs` |
| `VICTORY_BENCH_FILE=<file>` | unset | Test-only read benchmark. | `dstorage.rs` tests |
| `VICTORY_MAP_THREADS=<n>\|auto` | 8 | Threads that map a group of pages side by side (1..32). | `crates/victory-hle/src/modules/xmem.rs` |
| `VICTORY_MAP_LEAN=0` | on | Split and commit on every mapping change, as before. | `xmem.rs` |
| `VICTORY_SECTION_WINDOW=0` | on | A view per DirectStorage write instead of one kept view of the section. | `xmem.rs` |
| `VICTORY_STREAM_NOTICE=0` | on | Do not tell `victory-gpu` which pages DirectStorage wrote. | `xmem.rs` |
| `VICTORY_STREAM_PRIORITY=0` | on | Do not lift a below-normal streaming thread to above normal. | `xmem.rs` |
| `VICTORY_PAGE_WRITES` | (read in `victory-gpu`) | What is done when pages were written; named in `xmem.rs` comments. | `crates/victory-gpu/src/descriptor/host.rs` |
| `VICTORY_ONE_PAD=0` | on | Each host pad is its own device, instead of one pad always there. | `crates/victory-hle/src/modules/gameinput.rs` |
| `VICTORY_KEYS=<s>:<key>,...` | unset | Diagnostics: scripted key presses. | `gameinput.rs` |
| `VICTORY_WINDOW_MAX=WxH` | the work area | A smaller limit for the title's window. | `crates/victory-hle/src/modules/lifecycle.rs` |
| `VICTORY_RELAUNCHED_FROM` | set by us | Handle of the process a relaunch replaces. | `lifecycle.rs` |
| `VICTORY_XMA_DUMP=<dir>` | unset | Save the first 32 XMA voices' contexts and input. | `crates/victory-hle/src/modules/audio.rs` |
| `VICTORY_AUDIO_CAPTURE=<file.wav>` | unset | Diagnostics: record the process's audio. `VICTORY_AUDIO_CAPTURE_OF` is set for the child. | `crates/victory-hle/src/modules/audio/capture.rs` |
| `VICTORY_HOST_LOCK_CHILD`, `VICTORY_SAVE_CHILD`, `VICTORY_SAVE_OP`, `VICTORY_SAVE_STOP` | unset | Test-only child processes. | `host.rs`, `xgamesave.rs` |

Also relevant, read in `victory-gpu`: `VICTORY_COPY_BESIDE` (`HostPolicy::resolve`
swaps `memcpy`/`memmove`/`memset`).

## Traps

- **Registry order.** `lookup` returns the first module in `modules::all()`
  order that serves the DLL and lists the name. The duplicate-name test
  (`export_names_are_unique_per_dll_pattern`) checks inside one module only,
  so the same name in two modules that serve the same DLL silently picks the
  earlier one.
- **Exact names.** `hle_export!` uses the Rust identifier, so the function
  must be named exactly like the import (hence `#![allow(non_snake_case)]`).
- **Ordinals.** Binding looks ordinals up as `#n`; `GetProcAddress` on a
  pseudo handle refuses ordinals. `gameinput.dll` is exported only as `#1`.
- **`find_module` uses `HostPolicy::default()`**, not the resolver's policy,
  so `--strict` does not change what `LoadLibrary` returns for host DLLs.
- **Host DLLs load from System32 only** (`LOAD_LIBRARY_SEARCH_SYSTEM32`;
  `crates/victory-hle/tests/no_dll_planting.rs`). The module cache's lock is not held while
  loading: a title TLS callback runs under the OS loader lock and may look a
  function up (test `loading_a_host_dll_does_not_block_thread_attach`).
- **A host-resolved address may not be the host's** (`titlewrite::import`,
  `timeline::import`), so do not assume `HostPolicy::resolve` equals
  `GetProcAddress`.
- **Narrow paths go to the host's wide function**, never to our own wide
  wrapper, which would rewrite a host path a second time when its letter is
  mounted too (`files.rs` module doc).
- **Kernel writes into guarded memory fail instead of faulting.** Any new
  import through which the kernel writes title memory needs
  `victory_gpu::kernel_writing` around the host call, as `ReadFile` and
  `fread` do.
- **Runtime objects are static** and reference counting is a no-op. A method
  slot that returns `bool` or a value must not be left as `missing`, which
  returns an `HRESULT` (its doc says so).
- **`QueryApiImpl` must leave `out` untouched on failure** (tested in
  `title_stub_loads_the_runtime_dynamically`).
- **Process exit.** `kernel::exit` calls the host CRT's `exit` (`crt_exit` in
  the Windows `host` module), whose `atexit` list runs the loader's
  `detach_all` (`victory-loader::threads`).
- **Saves and storage are data, not cache.** They live under the data
  directory (`Title::storage`, `Title::saves`), never under `cache_dir`.

## Origins

Nearly all of this crate is VIctory by Marvis (`f4504f6` initial commit,
`754619f`, `2849477`), with many later contributors in the history
(`git log -- crates/victory-hle`). `rpcrt4.rs` was added by Nixitov
(`efbf107`). `image_watch.rs` (`bd80bbe`) is this fork's. Changes this fork
made to existing files name their source in their commit messages.

## Where to start for common changes

- A missing import or runtime area: the recipe above; the log line names
  the DLL and symbol, or for runtime areas the provider and interface ids
  (`runtime API area not implemented`) and for missing slots the byte offset.
- A file the title cannot find: `files.rs` (`translate`, the wrapper for that
  call) and the mounts in `apps/victory/src/run.rs`.
- Memory or page mapping: `xmem.rs`. Streaming reads: `dstorage.rs`, with
  `VICTORY_DSTORAGE_LOG` and `VICTORY_DSTORAGE_TRACE`.
- Sound: `audio.rs` (XMA), `apu.rs` (Opus), `dsp.rs` (effects).
- Input: `gameinput.rs`. Window, closing and relaunch: `lifecycle.rs`.
- What may reach the host: `host.rs` (`HostPolicy::default`) and `net.rs`.
