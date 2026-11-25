# victory-gpu: title memory, shadows, write tracking and textures

## Purpose

On the console the CPU and the GPU share one memory; here the title's memory is
ordinary process memory and the card has its own. This part of `victory-gpu`
keeps host copies ("shadows") of the title memory that draws and dispatches
read, finds out what the title's CPU wrote since a copy took it (write watch,
page guards, faults), brings the changed bytes to the card at submission, and
puts what shaders wrote back into title memory (the relay and the landing). It
also makes the host copies of title textures, detiles them and uploads them.

Callers: the list translators and draw code (`draw.rs`, `descriptor/host.rs`,
`list.rs`: see [victory-gpu-draw.md](victory-gpu-draw.md)) ask for copies and
declare reads and writes; the submission thread and the relay thread
(`host.rs`, `host/*.rs`: see [victory-gpu-host.md](victory-gpu-host.md)) run
the refreshes, the sends to the card and the landings; the runtime
(`victory-hle`, mostly `modules/xmem.rs`, `files.rs`, `dstorage.rs`) tells
`watch.rs` and `inplace.rs` when it maps, unmaps or writes title memory.

Background reading: [docs/findings.md](../findings.md) ("The one idea behind
most of it"), [docs/fork-ray-tracing.md](../fork-ray-tracing.md) ("How title
memory by address is held" for `memory.rs`).

## How it works

### 1. How title memory the GPU addresses is held

A title GPU address is a CPU address (`resource.rs` module doc: heaps and
resources are host memory). Shaders on the host cannot read it, so every range
a draw names is mapped to a host buffer, a `shadow::Shadow`. There are five
kinds, all the same type:

- **Title buffer copies.** `resource.rs` registers each title buffer with
  `shadow::register(resource, start, len)` in the `BUFFERS` map
  (`RwLock<BTreeMap<(start, resource), Entry>>`). `shadow::find` /
  `find_any` / `lookup` find the innermost buffer that holds an ask and make
  its copy on first use: a `host.shared_buffer` of the buffer's size rounded
  up to 64 KiB (system memory the card reads over the bus). A title address
  inside the buffer is the same offset in the copy (`Shadow::gpu`).
- **Card copies** (`shadow::cards`, `VICTORY_CARD_BUFFERS`, on). A buffer of
  at least `card_limits().0` bytes gets two sides: `Shadow::buffer` is in the
  card's own memory (what shaders see), and `Card::staging` (what
  `Shadow::data` maps) is the CPU's side. `card_room_for` makes one only while
  the card has room (80 %, or 97 % with `card_room_kept_on`) and while the
  sum stays under `card_limits().1`. The limits scale with the adapter's
  budget (`default_card_limits_mb`: on a 24 GB card 1 MiB up to a quarter of
  the budget, at most 8 GiB). Copies with two sides are kept in `CARDED`.
- **Windows outside buffers.** Memory no buffer object holds (the title's own
  allocator's vertices, constants) gets 4 MiB windows (`WINDOW`, finer with
  `VICTORY_WINDOW_KB`), kept in `LOOSE` by (start, length) (`shadow::loose`).
  An ask with no length (a root view) keeps the 4 MiB grid. Windows nothing
  asked for in a while go (`shadow::let_go_idle`). Optional variants: windows
  on the card (`window_card_for`, `VICTORY_CARD_WINDOWS_MB`), in CPU-mapped
  card memory (`window_mapped_for`, `VICTORY_WINDOWS_BAR_MB`).
- **A view's own bytes.** `shadow::exact` makes a copy that starts at the
  view's address, for buffer views that do not start on an element of their
  buffer's copy (asked by `descriptor/host.rs`). Kept in `EXACT`.
- **In place** (`inplace.rs`, `VICTORY_INPLACE`, off). `inplace::place_for`
  opens committed private title memory as a host heap
  (`OpenExistingHeapFromAddress1`) and places a buffer on it; the `Shadow`
  then has `place: Some(..)` and every copy/compare method returns at once
  (`Shadow::update`, `gpu_writes`, `copy_back_with` check `place`).

Memory read by raw 40-bit address (the ray-tracing structure builder) is
`memory.rs`: a directory per 64 KiB page, arenas of 64 KiB pages, each arena a
`shadow::by_address` copy with `changes_only` set (only dwords that changed go
up). `memory::ready` is called by `draw.rs` before such a draw or dispatch. The
rules are in the module doc and in fork-ray-tracing.md; this map does not
repeat them. `upload.rs` hands out the lists' upload memory (`Upload::new`)
from arenas reserved in [4 GiB, 2^37) with `MEM_WRITE_WATCH`, each registered
as a title buffer so that shaders can find it by address.

Recording threads look copies up many times per draw; `lookup` keeps a
per-thread cache of recent answers (`FOUND`, invalidated by the
`BUFFERS_CHANGED` counter; `VICTORY_QUICK_FIND`).

### 2. Catching the title's CPU writes (`watch.rs`)

Two kinds of title memory, two mechanisms:

- **Write-watched memory** (the runtime's pools reserved with write watching).
  `kernel_written` calls `GetWriteWatch` with reset, which reports and clears
  the dirty 4 KiB pages. Because asking clears the bits and copies overlap,
  every answer goes into the **journal** (`JOURNAL: RwLock<Journal>`):
  64 KiB page number to the number of the asking that last found it written
  (`next_asking`), plus which 4 KiB parts were written (`parts`,
  `VICTORY_PAGE_PARTS`). A copy keeps, per page, the asking after which it
  last took it (`Shadow::copied`); a page is stale if written later. While one
  thread asks about pages, `ASKING` (a list of held ranges plus a condvar)
  keeps others from asking about the same pages; it is taken before the
  journal's lock, never under it.
- **Physical-page views** (the runtime's sections; cannot be write-watched).
  `guard` makes the 64 KiB blocks it hands to a copy read-only
  (`protect_here`, `VirtualProtect`), noted in `Guards.protected` and
  `Guards.clean`. A title write faults into the vectored handler
  `on_write` (registered by `watch::install`, first in the process), which
  `open`s the block (read-write again, not clean) and continues the thread.
  The next asking reports every block that is not clean as written and guards
  it again. Blocks found uncommitted are noted `absent` (and `held` for
  placeholders, `VICTORY_KNOWN_HOLES`) and copied as zeros until the runtime
  maps them.

How a submission asks. `watch::ask_once` (called by `host.rs` when a
submission starts, `VICTORY_ASK_ONCE`, on) asks the kernel once for every range
its lists refresh (`ask_ahead` joins ranges closer than 8 MiB; `ask_rest` asks
the lone ones) and lets that asking stand (`Journal.kept`) until
`ask_once_end`. Every later `watch::writes(start, end, guarded)` inside it is
answered from the journal with no kernel call. Our own writes into title memory
while it stands are told by `watch::wrote`. `watch::nothing_new` answers
"nothing changed" with no lock but the journal's read lock, from the
`PUBLISHED` bits (lock-free bitsets `Bits`: `PUBLISHED`, `PROTECTED`,
`BESIDE`) and the copy's notes; the host uses it to skip whole refreshes
(`VICTORY_LIGHT_REFRESH`).

The cheaper ways of catching writes that README.md lists live here:

- **One view opened at a fault** (`open_one_view`, `VICTORY_OPEN_ONE_VIEW`,
  on). The title shows one physical page at several addresses
  (`inplace.rs` `Views.pages`). A block is clean only while every view of its
  page is read-only (`cover`, `protect_alias`, `Guards.aliased`). A fault
  opens the view written and marks the others not clean (they stay
  read-only); without the switch it opened all of them. Pages shown in more
  than `MANY_VIEWS` (64) places are guarded one address at a time.
  `VICTORY_VIEW_GUARD=0` turns the cross-view guard off.
- **Blocks that rest** (`rest`, `Guards.opened`, `Guards::rests`,
  `VICTORY_GUARD_REST`, 3,000 us). A block a fault opened is not guarded
  again for that long; it counts as written at every asking meanwhile, so
  nothing is missed.
- **Stores and copies done beside the guard** (`titlewrite.rs`, both off unless
  asked). `titlewrite::store` is called by `on_write` for a fault in a guarded
  block: it reads the faulting instruction, decodes it (`titlewrite/decode.rs`
  `decode::store`: moves, ALU ops with memory, xchg/xadd/cmpxchg, vector
  stores), and either skips it if it would leave memory as it is
  (`Store::same`, `watch::nothing_put`), or performs it through our own view
  of the same physical pages (`watch::put_beside` with
  `inplace::beside_page`), sets `BESIDE` for every view of the page and
  advances `Rip`. The block stays read-only and clean in the guards' eyes;
  the next asking takes the `BESIDE` bit and reports it written. A block takes
  at most `titlewrite::MOST` (32) units (a store counts 8) before it is opened
  normally; places in the code that fill blocks (`SITES`, `sparse`) are not
  done beside. `titlewrite::import` replaces the title's `memcpy`, `memmove`,
  `memset` imports (called by `victory-hle/src/host.rs`) with versions that do
  the guarded parts beside the guard (`in_pieces`). `titlewrite::put` is the
  same for descriptor records we copy for the title (`descriptor.rs`).
- **Places that look before they write.** `titlewrite::learn` counts, per
  faulting instruction address, how often a store changed nothing; one that
  mostly does and that the loader can patch (`Store::can_look`) is appended
  to the file `VICTORY_WRITE_SITES` names (image file name and offset). At the
  next start `victory-loader/src/write_sites.rs` rewrites those instructions
  to read first (called from `apps/victory/src/run.rs`). `learn` runs only
  inside `titlewrite::store`, so the file is only written with
  `VICTORY_STORE_BESIDE=1`.

What the runtime tells (all `pub fn` in `watch.rs`, re-exported in `lib.rs`):
`title_memory_remapping` (returns a `Remapping` write guard on `MAPPING`, held
across a whole unmap-and-map), `title_memory_remapped`,
`title_memory_unmapping`, `title_memory_splitting`, `title_memory_freed`,
`title_memory_allocated`, `kernel_writing` (a file read into title memory: its
blocks are opened and left alone, `Guards.kernel`, until the returned
`KernelWrite` drops), `writing`, and `title_memory_written` (DirectStorage
wrote physical pages through the runtime's own view; what is done is
`page_writes`, `VICTORY_PAGE_WRITES`). `inplace::title_memory_section`,
`title_memory_view` and `title_memory_section_gone` keep `inplace.rs`'s record
of views (`VIEWS`), and `title_memory_view` calls `watch::views_mapped` so
guarded aliases are opened.

Readers of title memory in place hold `watch::title_memory_reading` (the read
side of `MAPPING`) so no page goes away under them; `title_memory_reading_now`
is the non-waiting form. A failed read or write that may have met a change of
mapping is retried after it with `watch::again_after_change`. A title thread
that faults on a page that is away only for such a change waits in
`came_back`.

### 3. Getting changed bytes to the card

At record time a draw calls `Shadow::refresh(list, address, len)`, which only
queues the range on the list (`HostList::defer_refresh` in `host.rs`). At
submission the host's `Batch` (in `host.rs`) runs, per range,
`Shadow::nothing_new` and then `Shadow::update`:

- `update` cuts out ranges shaders of a submission still in the air may be
  writing (`Shadow::pending`) and calls `update_free` on the gaps.
- `update_free` asks `watch::writes`; if the memory is neither write-watched
  nor guardable it marks the copy `NOT_WATCHED` and copies the whole range
  every time (`Shadow::copy`).
- `copy_stale` walks the pages, takes those never taken or written since,
  narrows them to the 4 KiB parts written (`by_parts`), and copies them with
  `take_all` (several threads via `shadow::wide_each` when a refresh is over
  1 MiB). It also notes the range as taken whole (`Shadow::ranged`) so pages
  at the ends are not copied again each time.
- `Shadow::take` copies into the CPU side. For a copy with two sides it marks
  the copied runs in `Card::dirty` (`card_changed_runs`); for `changes_only`
  copies (and with `VICTORY_SEND_CHANGED`) it compares dword by dword
  (`dword_runs`) and marks only those. Pages only a landing wrote are compared
  4 KiB at a time and not sent again (`take_landed`, `VICTORY_LAND_ECHO`).
- Copies go through `crate::host::copy_mapped` / `read_into`, which use
  `safecopy::copy` (one `rep movsb` whose fault is turned into a failed copy
  by `on_write` via `safecopy::failing_exit`; `VICTORY_SAFE_COPY=0` goes
  through the kernel instead).

Before the lists run, `Host::card_before` (`host.rs`) takes
`shadow::card_dirty(sent_as)` (every carded copy's dirty runs, numbered by the
send that takes them when the relay is on, `Card::sent`) and copies them from
the CPU side to the card side. A copy with one side is read by the card
directly, so there is nothing to send.

Ranges written into while shaders may be writing the same copy (a submission
not yet landed) are handled by `Shadow::stale_under_writes` and
`merge_under_writes` (called from `host.rs`): title bytes that changed are
put into the copy and into its reference (snapshot or CPU side) dword by
dword, so the compare after the card ran takes them for the title's. The
pages are compared on the `victory-under` pool (`land::each_under`,
`VICTORY_UNDER_WIDE`, `VICTORY_UNDER_THREADS`).

### 4. Getting what shaders wrote back to title memory

Declaring. A draw or dispatch with unordered-access views calls
`Shadow::gpu_writes`, which queues the range (`HostList::defer_gpu_write`). At
submission, after the range's refresh, the host calls `Shadow::writing`:

- the range joins `Shadow::pending`, so no refresh copies title memory over it
  until it is done with (`copy_back_done`); with the relay each submission's
  declaration is counted in `Flights`, so a range two flights declared stays
  closed until both landed;
- a copy in the PC's memory takes a snapshot of the declared bytes into
  `Shadow::before` (a pooled buffer, `pooled` / `give_back`,
  `VICTORY_SNAPSHOT_POOL`); with `wide_back` (on) the snapshots are only noted
  in `SNAPS` and taken together by `shadow::take_snapshots` just before the
  submission goes to the card, on the `wide` worker threads;
- a card copy takes no snapshot: the compare runs on the card (a compute pass
  of ours that hashes 64 KiB pieces against `Card::before`; it lives in
  `host.rs` / `host/card_steps.rs`, not here).

`picked.rs` (`VICTORY_PICKED_WRITES`, off) adds declarations for buffers that
shaders write through views picked from the heap by a computed index: buffers
once found written so are declared each time, and a slice of the rest
(`VICTORY_PICKED_MB`) is compared each frame (`picked::declare`,
`picked::with`).

Landing. With the relay (`shadow::relay`, `VICTORY_RELAY`, on, stage 2; the
thread itself is `host::relay_worker`) a thread of the host lands flights in
order as the card finishes them:

- card copies: the host fetches the pieces the card's compare flagged, orders
  them with `land::ordered` (by 64 KiB block of title memory; pieces of one
  copy in a block joined), cuts them into jobs of about 256 KiB that never
  split a block (`land::jobs`), and runs `Shadow::land_card` on the
  `victory-land` pool (`land::each`, `VICTORY_LAND_THREADS`). `land_card`
  leaves out offsets still dirty or sent after the fetch (`Card::dirty`,
  `SentRuns::near`): those are title bytes the card did not have yet.
- PC-memory copies: `shadow::find_all` compares the copy with its snapshot by
  64 KiB piece on the `wide` workers, then `Shadow::land_back` (or, without
  `VICTORY_LAND`, `Shadow::relay_merge`) lands the differing pieces.
- Both end in `land::piece`, which calls `safecopy::merge` (AVX2 assembly in
  `safecopy.rs`): where the card's bytes differ from the reference, the
  dwords that then differ from title memory are stored into it, read just
  before the store; the reference takes the card's bytes. Title bytes the
  card did not change are never written.
- Where the title memory is a view of a section,
  `inplace::beside` gives our own view of the same pages and the piece is
  written there (`VICTORY_LAND_BESIDE`, on): no guard is opened. The job's
  `land::Notes::flush` then tells `watch::landed_beside`, which notes the
  4 KiB pages in `LANDED` and the 64 KiB blocks in the journal (after the
  bytes are in), and `Shadow::landed` marks the copy that landed them as
  already holding them (`VICTORY_LAND_TAKEN`). Where it is not, the piece
  goes through the title's address after `watch::landing` (which opens the
  guards).
- `land::Had` remembers what earlier fetches brought so pieces already held
  can be skipped (`VICTORY_LAND_SKIP`, off; counted either way).

Without the relay, the submitting thread waits for the submission and
`host.rs` `hand_back` calls `Shadow::copy_back_with` for each declared range
(`back_piece`, `put_changed`), then `copy_back_done`. (The doc of
`Shadow::copy_back` still names `Host::execute` as the caller.)

Texture read-back (`texback.rs`, `VICTORY_TEXTURE_BACK`, four water kinds by
default). `host.rs` asks `texback::wanted` for each texture a list writes; for
those it makes a `texback::Target` (holding a `Ticket` on the title
allocation) and, when the list closes, `texback::record` copies the texture
into a read-back buffer. After the list has run, `texback::land` (called from `host.rs`
`read_backs`) reads the
title allocation, tiles the texels into it in the console layout
(`Layout::tile` from `victory-formats`) and writes it back with
`copy_mapped`. A ticket is invalidated if the allocation is released,
replaced or remapped (`texback::released`, `replaced`, `remapped`, called from
`watch::title_memory_remapped` / `unmapping`); a stale one is skipped.

### 5. Textures

Making the host copy. Title textures are `resource.rs` `Resource`s with a
`Hosted` side. `resource::hosted` makes the host copy on first use
(`VICTORY_LAZY_TEXTURES`, on), as a `HostTexture` with its state, `serial`,
`holds` (what the last upload put in it) and `ahead` state. Variants it picks:

- the first level the title's views show, not the whole chain
  (`held_only`, `VICTORY_HELD_LEVELS`; `HostTexture::base`);
- a reserved texture whose levels get tiles one by one
  (`tiles.rs` `Tiled::make`, `VICTORY_LEVELS_ON_DEMAND`, off; needs tiled
  resources tier 2). Tiles come from 64 MiB heaps of 1,024 tiles
  (`tiles::take` / `give_back`); a level is mapped only in a submission-time
  step of a list that writes all of it; `tiles::show_from` keeps views off
  levels without memory;
- the colour twin of a depth texture shaders also write (`twin_texture`,
  `VICTORY_DEPTH_TWINS`);
- the host-written texture behind a view placed on its memory in another
  format (`resource::alias_of`, `texture::alias` copies through a `Through`
  buffer; `VICTORY_ALIAS_COPY`).

Textures and their staging buffers are cut from 64 MiB blocks (`pool.rs`,
`VICTORY_TEXTURE_POOL`, on) because one allocation costs about 310 us; a
piece given back is reused only after `REST` (2,500 ms), and pieces are cut
first from blocks every submission begun before the block was made has left
behind (`Ripe`, `VICTORY_RIPE_BLOCKS`, 12). `pool::fresh` says a texture was
made in the last second. `resource::room` / `room_lately` decide whether the
pass over textures picked by index (`draw.rs` `heap_textures`) may make more
host copies; `resource::let_go_idle` lets idle host copies of filled textures
go (`VICTORY_TEXTURE_SWEEP`).

Uploading. A draw that samples a texture calls `texture::upload` /
`upload_from` (from `draw.rs`, `descriptor/host.rs`, `list.rs`). Once per list
it gets or makes the texture's `Staging` (`texture::staging`: copyable
footprints of the host texture, a staging piece from `pool::staging`, the
console `Layout`), records `CopyTextureRegion` from the staging buffer into
every subresource, and queues the staging for the submission
(`HostList::defer_texture`). If the staging buffer was uploaded and is not
dirty, only the check is queued. At submission the host calls
`Staging::refresh` (several at once with `wide_textures` in `host.rs`):
`refresh_timed` decides whether the memory changed and, if so, calls
`Staging::detile` (`Layout::detile` from `victory-formats`) into the staging
buffer. The upload recorded earlier copies whatever the staging buffer holds
when the list runs, so detiling happens before the lists are executed.
Change detection:

- write-watched memory: `watch::writes(.., false)` and the asking number in
  `Last.asked`;
- view memory: `watch::writes(.., true)` (the guards); a hash (`digest`, a
  128-bit non-cryptographic hash) every eighth frame by address in turn, and
  at every check while the texture is "hot" (`HOT_FRAMES`,
  `VICTORY_TEXTURE_HOT`), to catch writes through other views
  (`VICTORY_TEXTURE_GUARD=0`: hash every time);
- partly mapped memory: read around holes and hashed at most every 30 frames;
- the read in place is done under `watch::title_memory_reading`; with
  `VICTORY_TEXTURE_NO_WAIT=1` it does not wait for the runtime's remapping
  and reads through the kernel instead (`refresh_unlocked`, `Staging::fill`).

`texture::unchanged` (`VICTORY_TEXTURE_SAME`) lets the caller skip an upload
whose bytes the host copy already holds (`HostTexture::holds`).

The copy queue of their own (`texture/ahead.rs`, `VICTORY_UPLOAD_AHEAD`; off
in code, set by `scripts/play.ps1` unless `-Safe`). The texture pass in
`draw.rs` calls `ahead::take` for a new texture picked by index (made in the
last second, common state, no upload asked yet, not tiled, not aliased; at most
96 MB on the way). That marks `HostTexture::ahead` `PENDING` and makes a
`Job`; `ahead::send` queues it. Two threads ("uploads ahead 0/1"), each with
its own allocator, list and fence on one `D3D12_COMMAND_LIST_TYPE_COPY` queue,
take batches of up to 16 MB or 64 textures, refresh (read and detile) their
staging buffers, record the copies, execute and wait. `finish` marks the
staging uploaded, the copy `DONE`, runs the jobs' `then` callbacks and calls
`descriptor::host::flip_now`. A list that is about to record an upload into a
copy with a job in flight waits for it in `ahead::listed` (up to 1 s). Slots
are shown only after the upload ran by `descriptor/host.rs` `show_after`
(`VICTORY_SHOW_AFTER`, off in code, set by `scripts/play.ps1`); that file is
in the draw map, and `ahead::there` is what it asks.

### 6. Threads and locks

| Thread | Runs here |
|---|---|
| Title threads | `watch::install`'s `on_write` (fault handler), `titlewrite::store`, the `memcpy`/`memset` replacements, the runtime's `title_memory_*` / `kernel_writing` hooks |
| Recording threads (list translators, `VICTORY_THIN`) | `shadow::find` / `find_any` / `exact`, `Shadow::refresh`, `gpu_writes`, `texture::upload_from`, `memory::ready`, `texture::ahead::take` / `send` |
| Submission thread (`host.rs`) | `watch::ask_once`, `Shadow::nothing_new` / `update`, `Shadow::writing`, `take_snapshots`, `Staging::refresh`, `card_dirty`, `merge_under_writes` |
| `shadow::wide` workers | `take_all` parts, snapshots (`copy_jobs`), compares (`find_all`, `find_pieces`), `wide_each` |
| `victory-land` pool (`land::each`) | `Shadow::land_card`, `land_back`, `land::piece` |
| `victory-under` pool (`land::each_under`) | pages of `merge_under_writes`; also used by `host/send.rs`, `host/card_steps.rs` |
| `victory-records` / `victory-views` pools (`land::each_view`) | descriptor work of `descriptor.rs` and `draw.rs` (only the pool lives here) |
| Relay thread (`host::relay_worker`) | `host.rs` `relay_land` / `relay_hand_back` drive the landing above; `texback::land` via `read_backs` (without the relay, `hand_back` and `read_backs` run on the submitting thread) |
| "uploads ahead 0/1" | `texture::ahead::Worker::work` |
| popwatch thread, book check thread | `popwatch::watch`, `book::walk` (diagnostics) |

Locks and their order, as the code documents them:

- `watch.rs`: `ASKING` before `JOURNAL`, never under it. `GUARDS` (one mutex
  for all guard state); the fault handler takes it, and `guard` drops and
  retakes it between runs so faults are not kept waiting. Windows is asked
  about holes with `GUARDS` not held. `PROTECTED` is set before a protection
  is made and cleared after it is undone, so who finds it clear has nothing to
  open. `MAPPING` (`RwLock<()>`): readers in place share it, the runtime
  takes it alone around a remap; "not to be taken twice by one thread, nor
  held across anything that maps title memory".
- `shadow.rs` per copy: `ranged` is locked inside `copied`; `flights` inside
  `pending`; `merging` (shared while pieces land, alone while title bytes go
  in under shader writes and in `placed_copy`) is taken ahead of `before`;
  `Card::sent` inside `Card::dirty`. Global: `BUFFERS` and `LOOSE`
  (`RwLock`s, readers share), `EXACT`, `CARDED`, `SNAPS`, `POOL`.
- `inplace.rs`: `VIEWS` (`RwLock`); `beside` must be called with
  `title_memory_reading` held.
- `texture.rs`: `STAGING` (map), then a `Staging`'s `last`; `texture::staging`
  asks `resource::host_tiled` before locking `STAGING` because the sweep locks
  in the other order.
- `memory.rs`: one `MEMORY` mutex for everything.

## Files

| Path | What it does | Key items |
|---|---|---|
| `crates/victory-gpu/src/shadow.rs` | Host copies of title buffers, windows, single views and by-address arenas; their refreshes, card sides, snapshots, compares, merges and landings; the `wide` worker threads. | `Shadow`, `Card`, `lookup`, `loose`, `exact`, `by_address`, `Shadow::update`, `copy_stale`, `take`, `writing`, `copy_back_with`, `relay_merge`, `merge_under_writes`, `land_card`, `land_back`, `card_dirty`, `take_snapshots`, `find_all`, `put_changed`, `Flights`, `relay`, `cards` |
| `crates/victory-gpu/src/watch.rs` | Which title memory the title wrote since a copy took it: write-watch journal, read-only guards on views, the fault handler, the runtime's mapping hooks. | `Journal`, `writes`, `ask_once`, `nothing_new`, `Guards`, `guard`, `open`, `cover`, `put_beside`, `landed_beside`, `install`/`on_write`, `title_memory_reading`, `title_memory_remapping`, `kernel_writing`, `page_writes`, `Tracked` |
| `crates/victory-gpu/src/resource.rs` | D3D12.X heaps, committed/placed/at-address resources and their sizes; title textures' host copies made on first use, twins, aliases, room rules and the idle sweep. | `create_committed`, `create_placed`, `create_at_address`, `texture_at`, `Resource`, `HostTexture`, `Hosted`, `hosted`, `host_copy`, `alias_of`, `room`, `let_go_idle`, `held_only`, `copyable_footprints` |
| `crates/victory-gpu/src/memory.rs` | Title memory read by raw address (ray-tracing builder): directory, arenas, faults read back per frame. | `Memory`, `Held`, `Packed`, `ready`, `Use`, `Ready`, `asked_sizes` |
| `crates/victory-gpu/src/inplace.rs` | Title memory opened as a host heap (in place); the record of the runtime's views of sections, aliases, and our own view of a whole section for writing beside guards; probe and census. | `place_for`, `Placed`, `title_memory_section`, `title_memory_view`, `views_gone`, `shown_elsewhere`, `pages_of`, `beside`, `beside_page`, `shown_bytes`, `Cover`, `probe`, `tick` |
| `crates/victory-gpu/src/copystat.rs` | Diagnostics build only: counters of copies, waits and memory for the frame line; memory map walk. | `C`, `add`, `mode`, `whole`, `range`, `barrier_test`, `log_memory`, `log_memory_map` |
| `crates/victory-gpu/src/safecopy.rs` | Copies, dword puts and merges into title memory that fail instead of faulting (hand-written AVX2 assembly with failing exits). | `copy`, `put_changed`, `merge`, `Merged`, `differ`, `failing_exit`, `on` |
| `crates/victory-gpu/src/land.rs` | Landing what shaders wrote on a pool of threads; piece order, jobs, notes for writes beside the guards; the generic `Pool` also used for descriptor work. | `on`, `piece`, `Notes`, `ordered`, `jobs`, `Had`, `each`, `each_under`, `each_view`, `Pool`, `stat`, `log` |
| `crates/victory-gpu/src/land/tests.rs` | Tests of the pool, piece order, `Had`, landing through either address, waiting for a change of mapping, landings beside refreshes; a cost measurement. | `the_pool_calls_every_number_once`, `landings_beside_the_title_and_its_refreshes_lose_nothing`, `what_landing_costs` |
| `crates/victory-gpu/src/titlewrite.rs` | The title's writes into guarded view memory done beside the guard: faulting stores and `memcpy`/`memmove`/`memset`; learns places for `VICTORY_WRITE_SITES`. | `import`, `store`, `put`, `in_pieces`, `learn`, `MOST`, `DONE`, `busiest` |
| `crates/victory-gpu/src/titlewrite/decode.rs` | Decodes the x86-64 store at a fault and runs it against the saved `CONTEXT`. | `store`, `Store`, `Store::same`, `Store::run`, `Store::can_look`, `Op`, `Alu` |
| `crates/victory-gpu/src/texback.rs` | Textures shaders write that the title's CPU reads (ocean height/displacement) copied back and tiled into title memory. | `kinds`, `wanted`, `Target`, `record`, `Back`, `land`, `Ticket`, `remapped` |
| `crates/victory-gpu/src/book.rs` | Diagnostics: who had which card memory, for device losses and the slot check. | `made`, `block`, `at_loss`, `check`, `decode`, `on` |
| `crates/victory-gpu/src/pool.rs` | Host textures and staging buffers cut from 64 MiB blocks, reuse after a rest, ripe blocks. | `texture`, `staging`, `StagingPiece`, `fresh`, `room`, `texture_room`, `Ripe`, `on` |
| `crates/victory-gpu/src/faultbook.rs` | Diagnostics: a binary log of guard faults and askings for 40 frames at chosen times. | `What`, `note`, `frame`, `on` |
| `crates/victory-gpu/src/popwatch.rs` | Diagnostics: presented frames read back and compared for textures that pop. | `take`, `sent`, `watch`, `judge`, `on` |
| `crates/victory-gpu/src/picked.rs` | Writes through views picked from the heap by computed index: found, declared and landed. | `picks`, `stores_picked`, `shown`, `declare`, `Ranges`, `found`, `with`, `lands` |
| `crates/victory-gpu/src/upload.rs` | Lists' upload memory in arenas below 2^37, registered as title buffers. | `Upload`, `reserve`, `block` |
| `crates/victory-gpu/src/tiles.rs` | Levels on demand: reserved host textures given tiles level by level. | `wanted`, `floor`, `Tiled`, `Tiled::make`, `give_on`, `claim`, `show_from`, `given_total`, `status` |
| `crates/victory-gpu/src/texture.rs` | Title-filled textures: staging buffers, change detection, detiling, upload recording, aliases through buffers. | `Staging`, `staging`, `upload`, `upload_from`, `Staging::refresh`, `refresh_timed`, `detile`, `digest`, `unchanged`, `alias`, `card_writes`, `committed`, `mapped_bytes` |
| `crates/victory-gpu/src/texture/ahead.rs` | First uploads of textures picked by index on a copy queue and two threads of their own. | `on`, `take`, `claim`, `send`, `Job`, `Taken`, `listed`, `Worker`, `finish`, `there` |

## Switches

Defaults are what the code does when the variable is unset. "toggled" means
read with `crate::toggled!` (on unless `0`; A/B-able in diagnostics builds).

| Switch | Default | What it changes | Read in |
|---|---|---|---|
| `VICTORY_CARD_BUFFERS` | on | big title buffers' copies get a side in the card's memory | `shadow.rs` |
| `VICTORY_CARD_BUFFERS_MIN_MB`, `VICTORY_CARD_BUFFERS_MB` | by VRAM budget | smallest buffer on the card, most bytes of card copies | `shadow.rs` |
| `VICTORY_CARD_BUFFERS_MIN_KB` | unset | smallest size in KiB (after fxceless c36971b2) | `shadow.rs` |
| `VICTORY_CARD_ALL` | off | every copy has two sides | `shadow.rs` |
| `VICTORY_CARD_ROOM_FOR_BUFFERS` | on | texture queue keeps room for card copies not made yet | `shadow.rs` |
| `VICTORY_CARD_WINDOWS_MB`, `VICTORY_WINDOWS_BAR_MB` | 0 | windows on the card / in CPU-mapped card memory | `shadow.rs` |
| `VICTORY_WINDOWS_OWN` | off | windows have two sides | `shadow.rs` |
| `VICTORY_WINDOW_KB`, `VICTORY_WINDOW_TAILS` | 4096, off | window grid; tails from the ask | `shadow.rs` |
| `VICTORY_RELAY` | on (stage 2) | copy-back on the relay thread; `0` off, `1` first stage | `shadow.rs` (and `host.rs`) |
| `VICTORY_WIDE_BACK`, `VICTORY_WIDE_SPIN` | on, 0 | snapshots and compares on worker threads; spin before sleep | `shadow.rs` |
| `VICTORY_SNAPSHOT_POOL` | 1024 | snapshot buffers kept | `shadow.rs` |
| `VICTORY_KEEP_SNAPSHOTS`, `_MB` | off, 256 | snapshots kept between submissions | `shadow.rs` |
| `VICTORY_LAND_ECHO` | on (toggled) | pages only a landing wrote are not sent up again | `shadow.rs` |
| `VICTORY_SEND_CHANGED` | off | only changed dwords go to card copies | `shadow.rs` |
| `VICTORY_EXACT_BACK` | on | shader writes put dword run by run | `shadow.rs` |
| `VICTORY_UNDER_WIDE` | on (toggled) | `merge_under_writes` pages on threads | `shadow.rs` |
| `VICTORY_REFRESH_CHECK` | off | samples copies after refresh, warns if stale | `shadow.rs` |
| `VICTORY_CARD_HASH_LEN` | on | card hashes cover the buffer's own bytes | `shadow.rs` |
| `VICTORY_QUICK_FIND` | on (toggled) | per-thread lookup cache | `shadow.rs` |
| `VICTORY_MEMORY_CARD_MB` | 4096 | most by-address memory on the card | `shadow.rs` |
| `VICTORY_PAGE_PARTS`, `VICTORY_TAKE_PARTS` | on | 4 KiB parts of a page kept and copied | `watch.rs` |
| `VICTORY_ASK_ONCE` | on | one write-watch asking per submission | `watch.rs` |
| `VICTORY_ASK_EARLY` | off | ms between early askings on a thread | `watch.rs` |
| `VICTORY_LIGHT_REFRESH`, `VICTORY_KNOWN_HOLES` | on | bits instead of askings; placeholders known | `watch.rs` |
| `VICTORY_OPEN_ONE_VIEW` | on (toggled) | a fault opens only the view written | `watch.rs` |
| `VICTORY_GUARD_REST` | 3000 us (toggled) | a just-opened block rests | `watch.rs` |
| `VICTORY_VIEW_GUARD` | on | all views of a page guarded together | `watch.rs` |
| `VICTORY_GUARD_ASK`, `VICTORY_GUARD_HOLES` | off, on | ask Windows every time; look past holes | `watch.rs` |
| `VICTORY_PAGE_WRITES` | `guards` (bit 1) | what a DirectStorage page write does | `watch.rs` |
| `VICTORY_SAFE_COPY` | on | copies by `rep movsb` with failing exit | `safecopy.rs` |
| `VICTORY_LAND`, `VICTORY_LAND_THREADS` | on, cores/4 (1-4) | landing on a pool | `land.rs` |
| `VICTORY_LAND_BESIDE`, `VICTORY_LAND_ANYTIME` | on, on | land through our own view; also during remaps | `land.rs` |
| `VICTORY_LAND_TAKEN`, `VICTORY_LAND_SKIP` | on, off | copy noted current after landing; skip pieces held | `land.rs` |
| `VICTORY_UNDER_THREADS`, `VICTORY_VIEW_THREADS` | 6, 4/6 | pool sizes | `land.rs` |
| `VICTORY_COPY_BESIDE`, `VICTORY_STORE_BESIDE` | off | copies / faulting stores beside the guard | `titlewrite.rs` |
| `VICTORY_SAME_RECORDS` | on (toggled) | descriptor copies skip equal bytes | `titlewrite.rs` |
| `VICTORY_WRITE_SITES` | unset | file of places that mostly change nothing | `titlewrite.rs` (written), `victory-loader` (applied) |
| `VICTORY_INPLACE`, `VICTORY_INPLACE_MB` | 0, 4096 | memory in place (1 windows, 2 buffers too) | `inplace.rs` |
| `VICTORY_INPLACE_PROBE`, `_MB` | 0 | probe and census (diagnostic) | `inplace.rs` |
| `VICTORY_BVH_SIZES_WAIT`, `VICTORY_BVH_TRACE`, `VICTORY_MEMORY_DUMP` | off | see fork-ray-tracing.md | `memory.rs` |
| `VICTORY_UPLOAD_LOW` | on | upload memory below 2^37 | `upload.rs` |
| `VICTORY_TEXTURE_BACK` | four water kinds | texture kinds copied back; `0` none | `texback.rs` |
| `VICTORY_LAZY_TEXTURES`, `VICTORY_HELD_LEVELS` | on | host copies on first use; from the first viewed level | `resource.rs` |
| `VICTORY_DEPTH_TWINS`, `VICTORY_ALIAS_COPY` | all | depth twins; aliased views | `resource.rs` |
| `VICTORY_TEXTURE_BUDGET_MB`, `VICTORY_TEXTURE_ROOM_PERCENT`, `VICTORY_CARD_ROOM_FOR_TARGETS_MB` | unset, 90, 3072 | room for host copies | `resource.rs` |
| `VICTORY_TEXTURE_SWEEP`, `VICTORY_TEXTURE_KEEP` | on, keep rule | idle host copies let go | `resource.rs` |
| `VICTORY_PLACED_FILLER`, `VICTORY_CARD_ACCOUNTING` | off | filler words in the key; accounting line | `resource.rs` |
| `VICTORY_TEXTURE_POOL`, `VICTORY_RIPE_BLOCKS` | on, 12 | texture blocks; blocks made ahead | `pool.rs` |
| `VICTORY_LEVELS_ON_DEMAND`, `VICTORY_LEVELS_MIN_KB` | off, 1024 | tiled host textures | `tiles.rs` |
| `VICTORY_TEXTURE_HOT`, `VICTORY_TEXTURE_GUARD` | on | hash hot textures; guard view textures | `texture.rs` |
| `VICTORY_TEXTURE_NO_WAIT`, `VICTORY_TEXTURE_FILL` | off, on | don't wait for remaps; fill from what is there | `texture.rs` |
| `VICTORY_TEXTURE_COPY`, `VICTORY_TEXTURE_ONCE`, `VICTORY_TEXTURE_SAME` | off, on, on | read through kernel; one-shot staging; skip equal uploads | `texture.rs` |
| `VICTORY_COMMITTED_QUERY`, `VICTORY_MAPPED_BY_VIEWS`, `VICTORY_MAPPED_BY_PAGE` | off, on, off | how "mapped" is asked | `texture.rs` |
| `VICTORY_UPLOAD_AHEAD` | off | first uploads on a copy queue of their own | `texture/ahead.rs` |
| `VICTORY_PICKED_WRITES`, `VICTORY_PICKED_MB` | 0, 32 | writes through picked views | `picked.rs` |
| `VICTORY_BOOK`, `VICTORY_SLOT_CHECK` | on with DRED, off | memory book; slot walk | `book.rs` |
| `VICTORY_FAULT_BOOK`, `VICTORY_FAULT_BOOK_AT` | unset, 150,210,270 | fault log | `faultbook.rs` |
| `VICTORY_POP_WATCH`, `VICTORY_POP_FOLDER`, `VICTORY_POP_ALL` | unset | pop detection | `popwatch.rs` |
| `VICTORY_COPY_MODE`, `VICTORY_BARRIER_TEST`, `VICTORY_MEMORY_MAP` | guarded, 0, on | timing experiments; memory map | `copystat.rs` |

Test-only reads: `VICTORY_D3D12_DEBUG` (`resource.rs` tests),
`VICTORY_TILES_TEST_CARD` (`tiles.rs` tests), `VICTORY_BENCH_FILE`
(`texture.rs` tests).

## Traps

- **A write the guards do not see is a stale copy.** Everything we write into
  title memory must be told: `watch::writing` / `landing` before (opens
  guards), `landed_beside` after (when written through our own view), or
  `watch::wrote` while a submission's asking stands. Writing title memory any
  other way leaves a block "clean" whose bytes changed.
- **`landed_beside` is noted after the bytes are in, never before** (its doc
  says why: a copy reading in between would get the older bytes with a newer
  number). `put_beside` sets `BESIDE` before clearing `PUBLISHED` for the same
  reason.
- **A block must be read-only at every view while it counts as clean**
  (`cover`). `views_mapped` opens guarded aliases when a new view appears;
  `title_memory_splitting` handles the runtime's unmap-and-remap of part of a
  view. The fix for the rare lost write (findings.md) is in this ordering.
- **`MAPPING`**: never take `title_memory_reading` twice on one thread or
  hold it across anything that maps title memory (deadlock with the runtime's
  writer). Hot paths use `title_memory_reading_now` and fall back.
- **The fault handler runs on title threads inside an exception**, taking
  `GUARDS` and `VIEWS`. `learn` spawns a thread for its file write rather
  than doing it there ("Not here, under a fault of the title's"). Our own
  code that faults on a guarded block while holding `GUARDS` would wait on
  itself. `safecopy` functions are leaf assembly so a stack walk works; their fault
  ranges (`*_at`..`*_after`) must cover every instruction that touches title
  memory.
- **`safecopy` uses AVX2 unconditionally** (`vpmaskmovd`, `vpblendvb`); there
  is no feature check in the file.
- **Never store the card's bytes back into the copy** while the card may be
  writing it for a later flight (`merge_piece` doc, `relay_merge`,
  `land_back`): only title memory and the reference are written.
- **A declared range is closed to refreshes until every flight that declared
  it has landed** (`Flights`); `copy_back_done` must be called with exactly
  the ranges one submission declared, each once.
- **Snapshots under `wide_back` are taken late** (`take_snapshots`), which is
  only right because a declared range's copy bytes cannot change before the
  submission goes (its doc lists why). Adding a refresh path that writes a
  declared range breaks this.
- **`KernelWrite` must live for the whole kernel write**: a refresh that
  guarded the block meanwhile would make the kernel's write fail silently (a
  short file read).
- **Tile mappings into a heap opened from title memory remove the device**
  (`inplace.rs` module doc: "never call them"). `VICTORY_INPLACE=2` must not
  be combined with `VICTORY_LATE_WAIT` (`inplace::level` doc).
- **Lock order in `texture::staging`**: `resource::host_tiled` is asked before
  `STAGING` is locked, because the sweep locks the other way.
- **Tests and the relay**: `shadow::relay` is off in the crate's own tests
  unless asked (`!cfg!(test)`), because tests read title memory right after a
  submission.
- **Pages a copy never took count as stale** even if never written
  (`stale_pages_in`, fxceless/victory-fork#206: missing geometry otherwise).

## Where to start for common changes

- A draw reads garbage or zeros from a buffer: `shadow::lookup` (which copy),
  `Shadow::update_free` / `copy_stale` (what was refreshed), then
  `VICTORY_REFRESH_CHECK=1`.
- The title's CPU sees old data that shaders wrote: is the range declared
  (`Shadow::gpu_writes` callers in `draw.rs`, `picked.rs`)? Then the landing:
  `land::piece`, `Shadow::land_card` / `land_back`, `put_changed`.
- A texture the title reads on the CPU: `texback.rs` `kinds`
  (`VICTORY_TEXTURE_BACK`).
- Too many write faults: `watch::guard`, `rest`, `open_one_view`;
  `titlewrite::store` and `VICTORY_FAULT_BOOK` to see which blocks.
- A new runtime mapping call: add the matching `watch::title_memory_*` and
  `inplace::title_memory_view` calls in `victory-hle` (`modules/xmem.rs` shows
  the existing ones) and hold `title_memory_remapping` across the change.
- Texture flicker or a late texture: `Staging::refresh_timed` (change
  detection), `texture::upload_from`; for first uploads, `texture/ahead.rs`
  and `descriptor/host.rs` `show_after` (draw map).
- Card memory pressure: `resource::room`, `shadow::card_room_for`,
  `pool.rs` spares, `resource::let_go_idle`.

## Origins

- `texback.rs`: from fxceless/victory-fork f5c03f3b, pull request 999 by
  Geckomacabre (895c0b8f, 04df4350), commit 23925a4; README also credits
  cxy0072019-sketch for the read-back.
- The guard over every view of a physical page (`watch.rs` `cover`,
  `views_mapped`, `inplace.rs` `Views.pages`): code by myokex, victory-amd
  c6dc4d8, after pull request #115 by Spexivus, its correction by joroyt and a
  part of VIctory #1122 by tightfall (commit 8dd30ab, fxceless).
- `watch.rs` fault handler race fix: Exorion, 4c1766c. The original write
  tracking: Marvis (2849477) and fxceless (578d459).
- `title_memory_written` follows upstream c75fdba (its doc says so).
- `resource::first_mip`, `texture::upload_from` "from the first level a view
  shows": after victory-amd 4436f8f1.
- `shadow::exact`: upstream VIctory 86a5ed6 does the same.
- `upload.rs` placement in [4 GiB, 2^37): as upstream VIctory 0f637570 and
  6d021fe3.
- Card buffers down to any size and hashes over a buffer's own bytes: after
  fxceless c36971b2 (commit 17c4717).
- `texture.rs` `detile` padding rule: VIctory 4fc9d70.
