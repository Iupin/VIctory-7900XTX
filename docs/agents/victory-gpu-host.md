# victory-gpu: host device, lists, queues and submission

Sibling maps: [victory-gpu-memory.md](victory-gpu-memory.md) (host copies of
title memory, write watch, resources, textures, landings) and
[victory-gpu-draw.md](victory-gpu-draw.md) (draw state, tables, pipelines,
indirect execution, replay). Background: [architecture.md](../architecture.md),
[findings.md](../findings.md) (why each of the paths below exists and what it
bought), [replay.md](../replay.md).

## Purpose

`victory-gpu` gives the title the console's D3D12.X objects (device, queues,
allocators, lists, fences, descriptor heaps, ...) as plain `#[repr(C)]` structs
with function tables, laid out the way the title's inlined code reads them, and
runs what the title records on one host D3D12 device. `victory-hle` hands the
title `device::create` and `device::configure_virtual_memory`; every later call
arrives through our tables. This map covers the part that turns those calls
into host command lists and submissions: the host device (`host.rs`), lists and
their thin recording, queues and the submission line, fences, frame pacing,
descriptor heaps and their host mirrors, and the switch, A/B and core-pinning
plumbing. What a draw or dispatch records, and how title memory gets to and
from the card, is handed off to the sibling maps.

## How it works

### Objects and tables

Every console object starts with `object::Object` (table pointer, bits,
reference count). `object::missing_table` fills a table with slots that log
their offset and caller once and fail, so a run names the next method to write.
`object::hand_out` boxes an object for the title; `object::release_boxed` frees
it at the last release. The device is a single static (`device::DEVICE`), and
its table `device::FUNCTIONS` is where queue, list, descriptor, fence, frame,
pipeline and resource creation are wired. `device::create` sets `NEWER` when the
title asks for `ID3D12Device11`; `device::newer()` then picks the newer
list and queue tables (two extra methods each, `list::newer_slot`,
`queue::newer_slot`). The DXGI side (`dxgi.rs`) is one static adapter, factory
and output, reached by interface query from the device.

Layouts the title reads are pinned by `const _: () = assert!(...)`: `List::put`
at 0x18 (the put pointer and four limits, all zero, so every inline packet
check fails and the title calls the method), `Queue::kind` at 0x10, `Device`
0x58 bytes. Do not reorder those structs' leading fields.

### The host device (`host.rs`)

`host::host()` makes the one `Host` on first use (`create`): optional debug
layer, GPU validation and device-removed data (diagnostics builds), the device
(or WARP, `warp_adapter`), the TDR registry values logged (`tdr::log`), the
host wave width told to the shader translator, one DIRECT queue, one fence and
event, RTV/DSV rings of 1024 (`Host::rtv`, `Host::dsv`), and the adapter for
budget queries. `None` means no host GPU: calls then do nothing and lists
never run. `host::fail` ends the run on a host failure (device removed, a list
that does not close) through the runtime's crash reporter (`on_failure`),
appending `removal_context` (the `TRAIL` of recent object work, what the
submitting thread was doing) and, with `VICTORY_D3D12_DRED`, `removal_data`.

`Host::submitted` (a `Mutex<u64>`, the last value signalled on `Host::fence`)
is **the submission lock**: one submission at a time is prepared, from the
first step to the send (and, for synchronous submissions, through the wait and
copy-back), because snapshots and declared shader-write ranges live per host
copy, not per submission.

### From a title call to the host: the three recording modes

1. **Direct (default).** `list::boxed` gives each title `List` a
   `host::HostList` (allocator + `ID3D12GraphicsCommandList`, plus per-list
   `Deferred` CPU work, scratch upload memory, assumed texture states, twins,
   indirect budget). The title's method runs on the title's thread and records
   into the host list right away: state setters go through `list::state` to
   `crate::draw::State` (draw map); copies, clears, barriers, writes and waits
   are in `list.rs`. Work that must happen on the CPU at submission time
   (copies of title memory, fills of upload memory, refreshes of host copies,
   declared shader writes, texture staging, bottom-of-pipe writes, read-backs)
   is queued on the host list as `host::Step`s by the `HostList::defer_*`
   methods (`defer_copy`, `defer_fill`, `defer_refresh`, `defer_gpu_write`,
   `defer_texture`, `defer`, `defer_host`, `defer_write`, `defer_read_back`).
   `list::buffer_copy` first tries `HostList::place_copy` (`host/place.rs`:
   the copy done on the card at its place in the list).
2. **Thin (`VICTORY_THIN=1`, `list/thin.rs`).** The title's lists get
   `THIN_FUNCTIONS`, whose recording methods only append an opcode and the
   arguments to a word stream (`thin::Front::put` / `put_with`), copying what
   D3D12 would read at the call (arrays, viewports, barriers, the RTV/DSV
   records, upload blocks). Every `CHUNK` (1024) words the stream is handed to
   the `victory-list-N` threads (`VICTORY_THIN_THREADS`, 8), which `translate`
   it call by call onto a list of ours (`back_list`, a normal direct-mode list
   the title never sees) through the same methods as mode 1. A `Recording` is
   `done()` when its CLOSE was translated. A recording reset or released
   without submission is `abandoned` and only UPLOAD, RESET and CLOSE are still
   replayed. Thin needs the submission line, so `thin::on` also turns on
   `queue::behind` (see below).
3. **Waits cut lists (`queue::cuts`, default on).** A recorded wait for a
   memory value (`list::wait_for`) closes the current host list into a `Part`
   and continues on a fresh one (`list::cut`); `State::replay` re-applies the
   draw state there. `list::detach_parts` turns the submitted lists into
   `(waits, host lists, order)` entries for the queue.

Host lists are pooled: `list::detach` / `detach_parts` take the host side out
of the title's list (a `Detached`, owning the upload blocks and the draw state
with its dynamic descriptor slots) and leave a fresh closed one
(`fresh_list`); after their run `list::recycle` resets and parks them in
`IDLE` (512 lists, 256 MiB of scratch at most; `VICTORY_LIST_POOL=0`: off).

### From a submit call to the card

`queue::execute` (the title's `ExecuteCommandLists`) chooses, in this order:

- `behind::on()` → `behind::submit`: the submission goes into the line of a
  thread of ours (async submission, below);
- `cuts()` → `submit_cut`: the lists before the first one with a wait go now
  (`Host::execute_late` with `late()`, else `Host::execute`); the rest is
  pushed as `Held::Value` waits and `Held::Run`s onto the queue's `held`, and
  `release_held` runs whatever is free;
- `waits() && split()` → `submit_split` (the older list-by-list holding);
- otherwise run at once unless the queue already holds something.

`queue::wait`/`signal` push `Held::Wait`/`Held::Signal` behind what a queue
holds; `release_held` walks the `WAITING` queues, runs what is met
(`fence::completed`, `ValueWait::met`, or `host::late_met`), and is called
after every signal and submission, never from inside one. `queue::present`
(without the line) settles flights first only at `late` stages 1-2
(`settle_for_present`); at stage 3 it only releases. Then it releases what is
held, forced past unmet waits (`VICTORY_WAIT_FORCE`, default on: no wait
outlives a frame), and calls `queue::show` → `Host::present`. With the line,
the present is a `Job::Present` instead.

`Host::execute(runs)` is the synchronous path (our own copies, clears,
read-backs, the present without stage 3). `Host::execute_late(lists, order)`
and `Host::execute_parts(.., parts)` are the asynchronous path. Both end in
**`Host::submit`**, whose stages are named by `timeline::span` strings:

1. take the submission lock; plan `card_steps` (`Host::card_steps`, stage 1-2
   of `VICTORY_CARD_STEPS`): steps that read memory a flight may still write
   are done on the card by a list of ours in front of the submission;
2. synchronous: `settle` everything; asynchronous: `late_blocked` says whether
   preparing reads what flights write, then `relay_wait` (only the flights
   that declared it, with `relay_exact`) or `settle`;
3. `descriptor::begin_sync_heaps` (records the title copied into heaps as
   bytes, on `victory-heap-sync`, dropped later = waited for);
4. `sweep` once a second when nothing is in the air (`pool::room`,
   `pipeline::trim_library`, the `let_go_idle`s of shadow/texture/resource);
5. `STARTED += 1`; one write-watch asking for every refresh
   (`watch::ask_once`); `stale_under_flights` → `settle_stale`;
6. refreshes in front (`Batch::refresh_front` with joined parts, else
   `Batch::refresh_ahead`), on the `shadow::wide` threads;
7. **the lists' steps**: `HostList::run_deferred` for each list, in order
   (copies through the kernel `copy_mapped`, fills, `Step::Placed`, refreshes
   batched per copy, `Run`/`Host` closures, `GpuWrite` = snapshot before the
   shaders write, texture staging); the list's `prelude()` (texture state
   transitions from each texture's real state, twin copies) is pushed before
   it; `Batch::fill`, `Batch::flush`;
8. `shadow::take_snapshots`; `picked::declare`; `declared_writes`;
   `card_before` (what changed on the CPU side of copies in the card's memory
   goes up first; a compare list `CARD_COMPARE` follows the lists);
9. drop the heap-sync guard (wait for it); `descriptor::make_waiting` (slots
   that waited for a texture's host copy);
10. send: with `late`, `ExecuteCommandLists` once per joined part, one
    `Signal`, `awake::sent`, and return `Sent` with the lock still held; else
    signal, wait on the event, then copy back (`card_back`, `hand_back`),
    `read_backs`, and the bottom-of-pipe writes (`Written::write`) in list
    order, `FINISHED`, `bury`.

### Flights, settling and the relay

With `late()` (`VICTORY_LATE_WAIT` stage 1-3; **stage 3 by default because the
relay is on by default**, see Traps) a sent submission becomes a `Flight` in
the global `LINE` (`send_late`), followed by its `Late::Write`s; a queue's
signal while flights are in the air becomes `Late::Signal` (`late_signal`).
The title learns that work is done (fences, bottom-of-pipe values) only when
the card has run it, never earlier. Flights are settled at the points listed
in the doc comment of `host::late` (a synchronous submission, a read of memory
flights may write, a stale refresh, an unmet value wait, a blocking fence
wait, all flights run, `late_most` lists in the air).

- Without the relay, `Host::settle` waits on the fence for everything in the
  air, copies back each byte once (`card_back`, `hand_back`), then does
  read-backs, writes and signals in line order. `late_helper`
  (`victory-late-wait`) settles when the card is done so a title polling a
  value is told without another call.
- With the relay (`relay()`, `VICTORY_RELAY`), `relay_worker`
  (`victory-relay`) lands flights as soon as the card has run them
  (`landable`), in line order, without the submission lock
  (`Host::relay_land`, `relay_card_back`, `RelayFetch` with its own list,
  fence and optionally the copy queue, `VICTORY_RELAY_COPY`). `settle` then
  only waits (`relay_wait` on `LINE_LANDED`). `late_helper` still resets
  landed lists (`land`, behind `bury_lag`) and calls `release_held`.
- `torn.rs` re-fetches pieces caught while a later flight wrote them
  (`VICTORY_FETCH_CHECK`); `send.rs` takes each submission's sends to card
  copies out of the CPU sides at preparing time (`VICTORY_SEND_OWN`);
  `upcheck.rs` checks that nothing sent up is older than the card's bytes.
  The merges themselves are in `shadow.rs` / `land.rs` (memory map).

### Asynchronous submission and joined parts (`queue/behind.rs`)

`behind::on()` is true with `VICTORY_ASYNC_SUBMIT=1` or `VICTORY_THIN=1`, but
only if `started()` finds `late_free_present()` and `queue::cuts()` (else a
warning and the title's thread submits). Then every submit, wait, signal and
present of every queue goes, in call order, into one `Lane<Job>`; the
`victory-submit` thread (above-normal priority, fast cores) runs
`Lane::serve`/`step`: the oldest head of any queue whose `ask` is met runs;
within a queue the order is the title's; a present runs only behind everything
given before it (or forced past unmet waits). `Job::Lists` stands for a thin
submission whose recordings are not translated yet; it is `unfold`ed into
`Job::Value`/`Job::Run` once they are `done()`. `Job::take_along` joins the
following `Run`s of the same queue into a `Job::Joined` while
`host::joinable` allows (`VICTORY_JOIN_PARTS`, `VICTORY_JOIN_LISTS`; needs
`ahead_of_writes`), absorbing waits that are met or that a joined part's
bottom-of-pipe write will meet (`host::will_meet`) and signals. `go` calls
`Host::execute_late` / `execute_parts` / `queue::show`.

Back-pressure: a submit call returns once at most `runs_most`
(`VICTORY_SUBMIT_DEPTH`, 64) are queued; a present once at most `frames_most`
(`VICTORY_SUBMIT_FRAMES`, 1) presents are. Anything that must observe earlier
submissions calls `queue::before_sync` → `behind::drain` (not on our own
threads: `no_drain_here`). `host::retire` counts `queue::queued()` so objects
the title releases wait for submissions still in the line.

### A frame's beginning and end

- Begin: `frame::wait` (device method) blocks to the next tick of a clock set
  by `frame::set_interval`; an origin wait hands out the next frame token
  (`frame::current`, used by `host::moment` and the idle sweeps). A wait for an
  event that already passed returns at once (`VICTORY_LATE_WAITS=1`: sleep to
  the next tick).
- End: `queue::present` → (line or direct) `Host::present`: the presented
  texture is copied onto the swap chain's back buffer in a list of ours
  (`Presenter::copy`, or one of `Presenter::ahead` sent as a flight with
  `late_free_present`, `Host::execute_own`), the diagnostics panel is drawn
  over it, `Present(0)`, then `Meter::frame` counts the frame and once a
  second writes the frame-rate line (it calls `late_line`, `split_line`,
  `descriptor::sync_line`, `queue::behind_line`, `card_clock_line`, ...). In
  diagnostics builds `queue::frame_ends` runs `ab::presented` there.

### Descriptors and heaps (`descriptor.rs`, `descriptor/host.rs`)

Title heaps are zeroed memory (or the title's own memory for a placed heap);
handles are addresses. Views are 32 bytes, samplers 16, RTV/DSV 128
(`increment`). `create_cbv/srv/uav/sampler/placed_*` write a 12-byte record
`[VIEW_TAG, i, generation]` into the slot (`descriptor::record`), where the
top bits of `generation` say whether `i` is an entry of the global `VIEWS`
table (views of resource objects, samplers; freed with the resource by
`forget`, under a new generation) or the record names a constant buffer or a
placed view whole (`VICTORY_VIEW_INLINE`). RTV/DSV records (`RtvRecord`) are
turned into host views from the `Host` rings when a command uses them.

Shader-visible view and sampler heaps get a `HostHeap` on first use
(`Heap::host`; writes before that wait in `pending`) with the same slot order,
because translated shaders index heaps directly. `descriptor::mirror(handle)`
→ `HostHeap::mirror(index, handle, for_draw)` makes host slot `i` from title
slot `i` when the record changed (`Made` keys per slot under 512 striped
locks, the lock-free `quick` check first). Buffer views and views of textures
without a host copy are left for later (a draw through a table, the texture
pass, `make_waiting`). The host heap holds `DYNAMIC` (2^18, up to 4x) extra
slots past the title's for per-draw tables (`HostHeap::dynamic`, `release`),
and is capped at the translator's null-slot limit (`view_heap_limit`). Records
copied into placed heaps as bytes are found through the write watch by
`sync_heaps` (pages read via the kernel, compared with `Copied`, made on
`land::each_view` threads). `ListViews` (`VICTORY_LIST_VIEWS`) caches views a
list's tables hold per thread and recording. Texture-slot timing
(`show_after`, `TextureQueue`, `Spans`) is documented in `descriptor/host.rs`
and overlaps the memory map.

### Threads of ours and core pinning (`cores.rs`)

`cores::keep_*_on_fast_cores` calls `SetThreadSelectedCpuSets` with the CPU
sets of the best efficiency class, if there is a slower class.
`VICTORY_FAST_CORES` (default 2) picks who asks: 1 `keep_on_fast_cores`
(threads a frame waits for one by one: `victory-submit`, `victory-relay`,
`victory-late-wait`, `victory-heap-sync`, `victory-aside`), 2 also
`keep_helper_on_fast_cores` (`victory-wide-N` in `shadow.rs`, the landing
helpers in `land.rs`), 3 also `keep_many_on_fast_cores` (`victory-list-N`,
some `land.rs` pools). Title threads are never asked. Other threads: `victory-card-clock`
(`card_clock_watch`), `victory-card-awake` (`awake.rs`).

### Switches (`switch.rs`) and A/B runs (`ab.rs`)

`switch::on(name)` = set and not `"0"`. The `toggled!` macro is "on unless
`0`", read once; in diagnostics builds, if `VICTORY_AB` names the switch, it
reads `ab::on(bit)` instead. Most switches are read directly with
`std::env::var_os(..)` in a `OnceLock`, each with its own rule (`is_none_or(v
!= "0")` for default-on, `is_some_and(v == "1")` for default-off), so an
unusual value is not interpreted alike everywhere.

`VICTORY_AB=<seconds>,<switch>,...` (diagnostics only, at most 8 switches,
seconds > 1): `ab::presented` (called at each present) starts phase
`elapsed / seconds`; switch n is on in phases whose number has bit n, and each
phase's frame-time mean, median and p90 are logged (first second left out).
Only code that asks `ab::pick` / `toggled!` follows it: in these files
`VICTORY_JOIN_PARTS` (on = 8 parts), `VICTORY_ASYNC_SUBMIT` (leaving it calls
`behind::leave`), `VICTORY_PLACED_ORDER`, `VICTORY_KEEP_SHOWN`,
`VICTORY_LIST_VIEWS`. `lib.rs` exports `switched` for other crates.

## Organisation of `host.rs`

In file order; refer to items by name.

| Section | Items |
|---|---|
| States, `Host`, retirement | `pc_state`, `texture_state`, `Host`, `RETIRED`, `STARTED`/`FINISHED`, `retire`, `retire_resource`, `bury_lag`, `bury`, `submissions` |
| Clocks and memory policy | `now_ms`, `coarse_ms`, `moment`, `long_ago`, `vram_keep`, `vram_idle_ms`, `ram_idle_ms`, `keep_freed`, `wait_resident`/`not_resident`/`mapped`/`resident` (`Aside` copy queue), `references`, `sweep` |
| Failure context and counters | `TRAIL`/`trail`, `SUBMITTING`, `removal_context`, `log_removal_context`, `SUBMISSION_US`, `COPY_BACK`, `CardClock`, `PREPARING`, `MADE`, `ALIVE`, `LISTS`, `on_failure`, `fail` |
| Device creation | `HOST`, `host`, `made_host`, `create`, `Host::driver`, `warp_adapter`, `mod tests` |
| `HostList` and depth twins | `HostList`, `Prelude`, `lazy_prelude`, `States`, `TwinUse`, `PLANE_*` shaders, `PlanePass`, `compile`, `record_plane_pass`, `record_twin_copy` |
| Diagnostics helpers | `dred_on/light/any/faults`, `LIST_NOTES`, `card_budget`, `timing`, `name` |
| Scratch memory | `Counted`, `Chunk`, `Scratch`, `scratch_pool`, `Spare`, `SPARE`, `take_chunk`, `give_chunks` |
| Card copies | `CardWork`, `CARD_COMPARE` (HLSL), `card_pass`, `rehash_runs`, `wide_merge`, `CardRange`, `CardPlan` |
| Submission-time steps | `Step`, `Deferred` |
| Flights | `late`, `late_stage`, `late_exact`, `late_free_present`, `relay`, `relay_exact`, `relay_copy`, `QUEUE_ORDER`, `landable`, `late_most`, `late_writes`, `Sent`, `Flight`, `Late`, `Line`, `LINE`, `WHY_*`, `LATE_STATS`, `touches`, `join_ranges`, `flying`, `late_blocked`, blame (diagnostics), `blocked_by`, `stale_under_flights` |
| Joined parts | `joins`, `joins_asked`, `joinable`, `will_meet`, `split` |
| Fences and threads of the line | `late_signal`, `late_met`, `late_met_known`, `settle_now`, `settle_for_present`, `settle_for_fence`, `land`, `late_helper`, `late_threads`, `relay_wait`, `RelayFetch`, `relay_worker`, `RelayBack`, `late_line` |
| Submodules | `aside`, `awake`, `card_steps`, `indirects`, `panel`, `place`, `send`, `spans`, `torn`, `traffic`, `upcheck`, `stamps` |
| Copy-back | `join_copies`, `continues`, `declare_once`, `declared_writes`, `walls_trace`, `read_backs`, `hand_back` |
| `impl Host` (submission) | `mesh_shaders`, `depth_bounds`, `new_list`, `make_prelude`, `make_list`, `card_clock`, `card_sent`, `card_ready`, `make_card_work`, `card_before`, `card_results_low`, `card_done`, `make_relay_fetch`, `relay_card_back`, `relay_land`, `card_back`, `execute`, `execute_late`, `execute_parts`, `send_late`, `execute_own`, `settle`, `settle_stale`, `submit`, `removal_data`, `log_messages`, `rtv`, `dsv` |
| `impl HostList` | `close`, `refreshes_coming`/`take_coming` (`aside`), `reset`, `changed_something`/`epoch`, `transition`, `twin_use` and twin helpers, `texture_written`, `prelude`, `note_op`/`note_read`, `ran_indirect`, `bind_pipeline`, `defer_*`, `upload`, `scratch*`, `budget` and card budget, `indirect_scratch`, `batch_place`, `keep`, `first_use`/`used`, `defer`, `defer_host`, `defer_host_read`, `defer_texture`, `run_deferred` |
| Refresh batching | `next_epoch`, `ByCopy`/`AddressHash`, `SPLIT`/`split_line` (diagnostics), `wide_refresh`, `Batch`, `ahead_of_writes`, `refresh_front`, `front_spans`, `Touch`, `written_spans`, `wide_fills`, `wide_textures`, `Batch::{add, flush_one, refresh_front, refresh_ahead, fill, refresh_textures, flush, flush_all}` |
| Title memory access | `copy_mapped`, `read_sparse`, `read_pieces`, `read_into`, `read_there`, `NtWriteVirtualMemory`, `direct_writes`, `write_mapped`, `for_pieces` |
| Buffers of ours | `Host::queue`, `read_back_buffer`, `upload_buffer` |
| Present | `swap_format`, `Presenter`, `Meter` (frame-rate line), `Host::present`, `Host::presenter`, `title_window`, `legal_state`, `legal_for` |
| Card memory | `Host::video_memory`, `video_memory_groups`, `log_video_memory`, `shared_buffer`, `mapped_card_memory`, `mapped_card_buffer`, `uav_buffer` |
| Removal report, window | `last_notes`, `fault_in`, `indirects_around`, `in_flight`, `transition`, `window`, `make_window`, `pump` |
| Tests | `tests`, `relay_tests`, `late_tests`, `wide_refresh_tests`, `written_spans_tests`, `trail`, `spare`, `refresh_tests`, `batch` |

## Files

| Path | What it does | Key items |
|---|---|---|
| `crates/victory-gpu/Cargo.toml` | Dependencies (`windows`, `windows-sys`, blake3, zlib-rs, victory-shader/-formats/-pipeline); feature `diagnostics` gates capture, replay, the debug layer, A/B, panel, stamps. | `[features] diagnostics` |
| `crates/victory-gpu/build.rs` | Hashes the shader translator's sources, `shader_cache.rs`, `Cargo.lock` and the toolchain file (CR bytes dropped) into `VICTORY_TRANSLATOR`, so cached translations last until the translator changes. | `files`, `main` |
| `crates/victory-gpu/src/lib.rs` | Module list (many `cfg(windows)` / `diagnostics`-only) and the crate's few exports. Its doc still says `present` is planned; it exists in `host.rs`. | `on_failure`, `switched`, `frame_phase`, title-memory hooks re-exported from `inplace`/`watch` |
| `crates/victory-gpu/src/host.rs` | The host device, host lists, submission, flights and relay, refresh batching, title memory copies, present and frame line. See the section table above. | `Host`, `HostList`, `Step`, `Deferred`, `Host::submit`, `Host::settle`, `LINE`, `relay_worker`, `Batch`, `copy_mapped`, `write_mapped`, `Host::present` |
| `crates/victory-gpu/src/host/card_steps.rs` | `VICTORY_CARD_STEPS` 1/2: a submission's fills (and, at 2, title buffer copies) that read memory flights may still write are done on the card by a list of ours in front of the submission; plans the order and refuses what it cannot prove equal. | `stage`, `plan`, `holder`, `cut_for`, `Taken`, `Host::card_steps`, `Writer`, `copied_read` |
| `crates/victory-gpu/src/host/upcheck.rs` | Diagnostics `VICTORY_UPCOPY_CHECK=1`: compares on the card whether a send up to a card copy puts older bytes over the card's, at each send and at rest. | `on`, `note`, `Looks`, `order`, `reads`, `at_rest` |
| `crates/victory-gpu/src/host/panel.rs` | Diagnostics `VICTORY_PANEL=1` (F3 toggles): a box of numbers drawn onto the swap chain buffer from the frame-line counters. | `Panel::{new, frame, draw}`, `Counters` |
| `crates/victory-gpu/src/host/torn.rs` | `VICTORY_FETCH_CHECK=1`: hashes fetched pieces like `CARD_COMPARE` does and marks pieces caught mid-write as suspects to fetch again; AVX2 hash. | `Suspects`, `Check`, `hash`, `flagged` |
| `crates/victory-gpu/src/host/send.rs` | `VICTORY_SEND_OWN` (default on): sends to card copies come from memory of the submission's own, short runs as offset/value pairs scattered by one dispatch. | `own`, `goes_as_words`, `words_under`, `take`, `Own::record`, `Memory` |
| `crates/victory-gpu/src/host/place.rs` | `VICTORY_COPIES_IN_PLACE` (default on): title buffer copies done on the card at their place in the list, the destination declared as a shader write. | `HostList::place_copy`, `place_source`, `Placed`, `Waits`, `read_by` |
| `crates/victory-gpu/src/host/awake.rs` | `VICTORY_CARD_AWAKE=1`/`turns`: a compute queue kept busy with a tiny loop while submissions come, so clocks stay up. | `sent`, `start`, `Awake`, `run` |
| `crates/victory-gpu/src/host/spans.rs` | Diagnostics `VICTORY_CARD_SPANS=1`: timestamp lists around each submission for card busy time. | `Host::span_lists`, `Spans`, `stamped`, `line` |
| `crates/victory-gpu/src/host/stamps.rs` | Diagnostics: per-submission begin/end timestamps for `crate::timeline`. | `timeline_ready`, `take`, `lists`, `timeline_stamps` |
| `crates/victory-gpu/src/host/indirects.rs` | Per-list record of indirect executions (pipeline, user data) for the device-removed report with DRED light. | `Indirects`, `Shared`, `nth`, `before`, `slots` |
| `crates/victory-gpu/src/host/aside.rs` | `VICTORY_HEAP_PASS_ASIDE` (default on): the draw code's pass over heap buffer views runs on `victory-aside`; its refreshes are spliced into the list's steps at close. | `begin`, `Coming`, `Refresh`, `WAITED` |
| `crates/victory-gpu/src/host/traffic.rs` | Diagnostics: bytes and 64 KiB pieces sent up / fetched per host copy, top eight every 5 s. | `moved`, `line` |
| `crates/victory-gpu/src/descriptor.rs` | Title descriptor heaps, view records and the `VIEWS` table, RTV/DSV records, descriptor copies, mirroring into host heaps, the byte-copied record sync. | `Heap`, `create_heap`, `record`, `View`, `VIEWS`, `view`, `forget`, `mirror`, `SHADER_HEAPS`, `sync_heaps`, `begin_sync_heaps`, `make_waiting`, `RtvRecord` |
| `crates/victory-gpu/src/descriptor/host.rs` | Host shader-visible heaps in title slot order: making slots from records, dynamic slots for tables, null filling, raw twins, slot-held resources, texture-slot queue and placed spans, per-thread `ListViews`. | `HostHeap::{new, mirror, dynamic, release, make_at, make_listed, copied_begin}`, `Made`, `Slots`, `DYNAMIC`, `show_after`, `Spans`, `TextureQueue`, `list_views` |
| `crates/victory-gpu/src/queue.rs` | Title command queues: execute/signal/wait/present, holding behind fences and memory values, `Written` bottom-of-pipe values, page mapping copies. Its doc ("waited for, so fence signals complete at once") predates flights. | `execute`, `submit_cut`, `submit_split`, `Held`, `release_held`, `before_sync`, `signal`, `wait`, `present`, `show`, `Written`, `copy_page_mappings` |
| `crates/victory-gpu/src/queue/behind.rs` | Async submission: one ordered line of all queues' work served by `victory-submit`, with joined parts and thin submissions unfolded when translated. | `Lane`, `Work`, `Job`, `on`, `started`, `submit`, `signal`, `wait`, `present`, `drain`, `queued`, `runs_most`, `frames_most` |
| `crates/victory-gpu/src/list.rs` | Allocators and lists: the `List` layout, direct recording onto host lists, cuts at waits, detaching and pooling host sides, copies/clears/barriers/read-backs, value waits and writes, method tables. | `List`, `Part`, `Detached`, `detach`, `detach_parts`, `recycle`, `fresh_list`, `cut`, `ValueWait`, `wait_for`, `write32/64`, `buffer_copy`, `METHODS`, `list_table` |
| `crates/victory-gpu/src/list/thin.rs` | `VICTORY_THIN=1`: list calls written as word streams and translated by `victory-list-N` threads onto lists of ours. | `on`, `wanted`, `Front`, `Recording`, `translate`, `plain!`, `MINE`, `table`, `submitted`, `start` |
| `crates/victory-gpu/src/frame.rs` | Frame pacing: interval clock, frame tokens, origin/vblank/flip waits. | `set_interval`, `wait`, `next_tick`, `current` |
| `crates/victory-gpu/src/fence.rs` | Fences: value readable at its address, events, signal; a null-event wait settles flights first. | `create`, `set`, `signal`, `completed`, `set_event_on_completion` |
| `crates/victory-gpu/src/device.rs` | The static device and its method table; feature answers; virtual memory config; `ID3D12Device11` → newer layouts. | `DEVICE`, `create`, `newer`, `FUNCTIONS`, `check_feature_support`, `configure_virtual_memory`, `removed_reason` |
| `crates/victory-gpu/src/dxgi.rs` | Static DXGI device, adapter (AMD vendor id), factory and output; the display mode list. | `device`, `adapter_description`, `display_size`, `modes` |
| `crates/victory-gpu/src/object.rs` | Object header, HRESULTs, interface ids, hand-out/release, private data, logging tables for unwritten methods. | `Object`, `This`, `Table`, `hand_out`, `release_boxed`, `answer`, `missing_table`, `slot` |
| `crates/victory-gpu/src/console.rs` | Which console model the title sees and its title memory sizes. | `Console`, `set`, `current`, `title_memory` |
| `crates/victory-gpu/src/switch.rs` | `switch::on` and the `toggled!` macro (A/B aware in diagnostics). | `on`, `toggled!` |
| `crates/victory-gpu/src/cores.rs` | Fast-core preference for our threads by stage. | `cpu_sets`, `fast_of`, `stage`, `keep_on_fast_cores`, `keep_helper_on_fast_cores`, `keep_many_on_fast_cores` |
| `crates/victory-gpu/src/ab.rs` | Diagnostics `VICTORY_AB`: switches flipped by phase at presents, frame times per phase. | `parse`, `pick`, `bit`, `on`, `presented` |
| `crates/victory-gpu/src/tdr.rs` | Reads and logs TdrLevel/TdrDelay/TdrDdiDelay; warns for levels 0 and 1. | `Tdr`, `log` |

## Switches

Read in the files above. "On"/"off" is the default when the variable is unset.
`scripts/play.ps1` sets several of them (noted).

| Switch | Default | What it changes | Read in |
|---|---|---|---|
| `VICTORY_THIN` | off (play.ps1: 1 unless `-Safe`) | list calls written down, translated by our threads; turns on async submission | `list/thin.rs` |
| `VICTORY_THIN_THREADS` | 8 (1-32) | translating threads | `list/thin.rs` |
| `VICTORY_ASYNC_SUBMIT` | off (on with `VICTORY_THIN`) | submissions prepared by `victory-submit`; A/B-able | `queue/behind.rs` |
| `VICTORY_SUBMIT_DEPTH` / `VICTORY_SUBMIT_FRAMES` | 64 / 1 | how far the title may run ahead of the line | `queue/behind.rs` |
| `VICTORY_JOIN_PARTS` / `VICTORY_JOIN_LISTS` | 1 (off) / 48 (play.ps1: 8 unless `-Safe`) | parts of submissions prepared as one; A/B-able | `host.rs` `joins_asked` |
| `VICTORY_LATE_WAIT` | stage 3 if the relay is on, else 0 (forced 0 with `VICTORY_GPU_TIMING`) | 1 flights; 2 title bytes merged under flights' writes; 3 present sent ahead | `host.rs` `late_stage` |
| `VICTORY_LATE_WAIT_LISTS` | 256 | host lists in the air before a settle | `host.rs` |
| `VICTORY_LATE_WAIT_WRITES` | on | `0`: a flight with bottom-of-pipe writes is settled at once | `host.rs` |
| `VICTORY_RELAY` | on, stage 2 (read in `shadow.rs`; play.ps1: 2) | landings on `victory-relay`; `1` waits for everything, `0` off | `host.rs` `relay_exact`, `shadow.rs` |
| `VICTORY_RELAY_COPY` | off (play.ps1: 1) | fetches over the copy queue | `host.rs` |
| `VICTORY_CARD_STEPS` | 0 (play.ps1: 2) | steps done on the card in front of the submission | `host/card_steps.rs` |
| `VICTORY_PLACED_ORDER` | off | placed copies block card steps only when behind them; A/B-able | `host/card_steps.rs` |
| `VICTORY_COPIES_IN_PLACE` | on | title buffer copies on the card at their place | `host/place.rs` |
| `VICTORY_SEND_OWN`, `VICTORY_UNDER_WORDS`, `VICTORY_SEND_BAR` | on, on, off | sends from per-submission memory; whole words under flights; send memory in card memory | `host/send.rs` |
| `VICTORY_CARD_REHASH_RUNS` | off (play.ps1: 1) | rehash touching pieces once | `host.rs` |
| `VICTORY_WIDE_MERGE` | off | merges of fetched pieces on threads (not with relay) | `host.rs` |
| `VICTORY_JOIN_COPIES` | off (play.ps1: 1) | contiguous title copies joined into one step | `host.rs` |
| `VICTORY_DECLARE_ONCE` | on | a declared write range noted once per list until title memory is written | `host.rs` |
| `VICTORY_WIDE_REFRESH` | on with >= 4 CPUs (`1`/`0` force) | refreshes spread over threads | `host.rs` |
| `VICTORY_REFRESH_AHEAD`, `VICTORY_REFRESH_FRONT` | on, off (on with joins) | refreshes ahead of / all in front of the steps | `host.rs` |
| `VICTORY_WIDE_FILLS`, `VICTORY_WIDE_TEXTURES` | on (with wide refresh) | fills / texture checks batched over threads | `host.rs` |
| `VICTORY_FILL_TITLE` | off | (see `fill_is_titles`) | `host.rs` |
| `VICTORY_COPY_ASK`, `VICTORY_DIRECT_WRITES` | off, on | ask `VirtualQuery` before copies; `NtWriteVirtualMemory` for writes | `host.rs` |
| `VICTORY_LAZY_PRELUDE`, `VICTORY_SCRATCH_POOL`, `VICTORY_INDIRECT_CARD_BUDGET` | on | prelude made on need; shared scratch chunks; budget words in card memory | `host.rs` |
| `VICTORY_WAIT_RESIDENT`, `VICTORY_KEEP_FREED`, `VICTORY_BURY_LAG` (64), `VICTORY_SWEEP`, `VICTORY_TEXTURE_KEEP` | on | residency waits, keeping freed memory, delayed drops, idle sweeps, VRAM keep rules | `host.rs` |
| `VICTORY_TRAIL` | on | the trail said at a device removal | `host.rs` |
| `VICTORY_WALLS_TRACE` | off (`1`/`2`) | log lines of what the title was handed | `host.rs` |
| `VICTORY_D3D12_DEBUG`, `_GBV`, `_WARP`, `_DRED` (`light`/`faults`) | off (play.ps1: DRED light) | debug layer, GPU validation, software adapter, device-removed data (diagnostics) | `host.rs` |
| `VICTORY_D3D12_BACKGROUND` | as is | `0`: driver background shader work off | `host.rs` |
| `VICTORY_GPU_TIMING` | off | each list timed alone (diagnostics); disables flights | `host.rs` |
| `VICTORY_CARD_AWAKE` | off (play.ps1: 1) | keep the card busy; `turns` alternates every 20 s | `host/awake.rs` |
| `VICTORY_CARD_SPANS`, `VICTORY_UPCOPY_CHECK`, `VICTORY_FETCH_CHECK`, `VICTORY_PANEL` | off | diagnostics above | `host/*.rs` |
| `VICTORY_HEAP_PASS_ASIDE` | on | heap view pass on `victory-aside` | `host/aside.rs` |
| `VICTORY_QUEUE_WAIT`, `VICTORY_SPLIT_WAITS`, `VICTORY_CUT_WAITS`, `VICTORY_WAIT_FORCE` | on | fence waits held; list by list; lists cut at waits; presents force held work | `queue.rs` |
| `VICTORY_GPU_LOAD_MS` | 0 | fake GPU time in title timestamp pairs | `queue.rs` |
| `VICTORY_LIST_POOL`, `VICTORY_READ_BACK` | on | host list pool; texture read-backs | `list.rs` |
| `VICTORY_WAIT_ASK`, `VICTORY_CUT_WRITES`, `VICTORY_COPY_OVER_TITLE` | off, off, (see `list.rs`) | `VirtualQuery` before value checks; cut lists behind value writes | `list.rs` |
| `VICTORY_HEAP_SYNC` (`0`/`all`), `_ASIDE`, `_EARLY` (4 ms) | on | byte-copied records mirrored; on `victory-heap-sync`; early rounds | `descriptor.rs`, `descriptor/host.rs` |
| `VICTORY_LENT_HEAP`, `VICTORY_VIEW_INLINE` | on | heap lent to lists without one; addresses named whole in records | `descriptor.rs` |
| `VICTORY_SHOW_AFTER` | off (play.ps1: 1) | a slot shows a new texture only after its upload ran | `descriptor/host.rs` |
| `VICTORY_DYNAMIC_SLOTS`, `VICTORY_KEEP_VIEWS`, `VICTORY_KEEP_SHOWN` (1, A/B-able), `VICTORY_QUICK_MIRROR`, `VICTORY_LIST_VIEWS` (A/B via `toggled!`), `VICTORY_RAW_VIEWS`, `VICTORY_VIEW_BYTES`, `VICTORY_VIEW_EXTENT`, `VICTORY_VIEW_COPY`, `VICTORY_SLOT_HOLDS`, `VICTORY_TABLED_TEXTURES`, `VICTORY_STALE_TEXTURES`, `VICTORY_TEXTURE_AWAY_MS` | see code | host heap slot rules | `descriptor/host.rs` |
| `VICTORY_LATE_WAITS` | off | frame waits sleep to the next tick | `frame.rs` |
| `VICTORY_DISPLAY_MODE` | 3840x2160 | the mode the console reports | `dxgi.rs` |
| `VICTORY_FAST_CORES` | 2 | core pinning stage 0-3 | `cores.rs` |
| `VICTORY_AB` | unset | A/B phases (diagnostics) | `ab.rs` |

## Traps

- **The relay and flights are on by default.** `shadow::relay()` is true when
  `VICTORY_RELAY` is unset (outside tests), so `late_stage` returns 3. The doc
  comments of `host.rs` (module: "every submission is waited for"),
  `host::relay` ("off by default"), `queue.rs` and `fence.rs` predate that.
  `VICTORY_GPU_TIMING` turns flights off; tests run without the relay.
- **Submission lock.** `settle_now`, `settle_for_*` and `Host::execute` take
  `Host::submitted`; never call them from inside `submit` or from code a
  submission's steps run. `relay_worker` never takes it, which is why
  `relay_wait` may be called with it held. `release_held` must not run inside
  a submission. `QUEUE_ORDER` is held from taking card-copy ranges until their
  list is on the queue.
- **The title is told late, never early, and in order.** Copy-back of shader
  writes, then read-backs, then bottom-of-pipe writes, then fence signals, in
  line order. `settle` settles everything in the air, not just the first
  flights (the merge writes CPU sides a later flight may also be writing);
  with the relay one thread lands in `LINE` order.
- **Lists are recorded on many threads and run in submission order**, so a
  list cannot know a texture's state: it records first/last states and its
  `prelude` transitions from the real state at submission. The same for
  depth twins (`TwinUse`). Code that records a transition must go through
  `HostList::transition`.
- **Drops wait.** Released host objects go through `retire`/`retire_resource`
  and are dropped `bury_lag` (64) submissions after the last that may use
  them, counting submissions still in the line (`queue::queued`). Landed host
  lists are reset only behind `bury_lag` (`land`). Removing that lag brought
  back device losses (see the comments).
- **`HostList::reset` assumes nothing of the list is executing**; prelude
  and scratch reuse rely on it.
- **Thin recording copies what D3D12 reads at the call.** A new recorded
  method needs an opcode in `plain!` or a `put_with` that copies its arrays;
  the test `every_recording_method_is_written_down` checks the `MINE` table
  against `METHODS`/`MORE_METHODS`. Objects named must live until the list
  ran.
- **Async submission prerequisites**: `late_free_present()` and `cuts()`;
  otherwise it silently falls back (warning). A panic on `victory-submit` or a
  `victory-list-N` thread ends the run (`host::fail`), since nothing would be
  sent again.
- **Waits**: `ValueWait::met` counts unknown comparisons and unmapped memory
  as met. A wait before any work in a list does not cut. At a present held
  work is forced (`VICTORY_WAIT_FORCE`). A list named twice in a cut
  submission runs once (logged).
- **Descriptor heap pointers** from `SHADER_HEAPS` are valid only while its
  read lock is held (`Heap::drop` takes the write lock). `Heap::host` is
  published only after `pending` writes are replayed. Record keys are never 0
  (generations start at 1). Title slots past the host cap read the null slot.
  Byte-copied records are visible only through the write watch of placed
  visible heaps, and must be mirrored before the lists go to the queue
  (`SyncedHeaps` is dropped in `submit` before sending).
- **Switch parsing is per site.** `switch::on` treats any value but `0` as on,
  many direct reads require exactly `1`. A switch named in `VICTORY_AB` no
  longer follows its variable, and only sites using `ab::pick`/`toggled!`
  follow A/B at all. `ab` exists only in diagnostics builds.
- **`Written::GpuTimestamp` is CPU time** at landing (100 MHz since start);
  the title's timestamp pairs say the card did nothing unless
  `VICTORY_GPU_LOAD_MS` is set.
- **Core pinning is a preference** the scheduler keeps unless no fast core is
  free; with one efficiency class nothing is asked.

## Where to start for common changes

- A new or changed list method: `list.rs` (`METHODS`, `MORE_METHODS`,
  `NEWER_METHODS`), then `list/thin.rs` (`plain!` or an opcode in `translate`
  and `MINE`). Draw/dispatch semantics: the draw map.
- New CPU work at submission time: a `Step` variant and a `HostList::defer_*`
  in `host.rs`, handled in `run_deferred`; then decide what `late_blocked`,
  `blocked_by`, `joinable` and `card_steps::does_of` must know about it.
- When flights settle or wait: `host::late` doc list, `late_blocked`,
  `stale_under_flights`, `Host::settle`, `relay_wait`, `send_late`.
- Queue ordering between title queues: `queue.rs` (`Held`, `release_held`) and,
  with async submission, `queue/behind.rs` (`Lane::step`, `Job::take_along`).
- Frame pacing: `frame.rs`; the present and the frame-rate line:
  `Host::present`, `Meter::frame`.
- Descriptor mirroring: `descriptor::record` → `mirror` →
  `HostHeap::mirror`; byte-copied records: `sync_heaps`; dynamic table slots:
  `HostHeap::dynamic`.
- A device removal to investigate: `host::fail`, `removal_context`,
  `Host::removal_data`, `host/indirects.rs`, `VICTORY_D3D12_DRED`.
- A new thread of ours: name it, and call the right `cores::keep_*` for its
  role.
- A new switch to measure in one run: read it with `toggled!` or `ab::pick`.

## Origins

From `git log` and source comments:

- `host.rs`, `list.rs`, `queue.rs`, `device.rs`, `dxgi.rs`, `object.rs`,
  `fence.rs`, `frame.rs` began in Marvisak/VIctory (first commit 754619f,
  author Marvis); `descriptor/host.rs` and `console.rs` from 2849477 (Marvis);
  `lib.rs` from f4504f6 (Marvis). All carry later commits of
  fxceless/victory-fork and its contributors and of this fork.
- `tdr.rs`: 3e3c491, author bvtterbroot. `switch.rs` (`0` means off):
  77582c3, author KoLolX3.
- `cores.rs`: `cpu_sets` and `keep_on_fast_cores` are fxceless/victory-fork's
  (`framelog.rs`, commit 0b0074e0), as its header says.
- `queue/behind.rs`: idea and argument from fxceless/victory-fork commit
  f85101b6. Joined parts (`host::joins`): fxceless/victory-fork commit
  39731795.
- `descriptor::sync_heaps`: upstream VIctory 86a5ed6 does the same.
  `device::removed_reason`: VIctory 13185f5.
- `list/thin.rs`, `queue/behind.rs`, `ab.rs`, `build.rs` and every file in
  `host/` were written in this fork.
