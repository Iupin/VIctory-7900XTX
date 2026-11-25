# victory-gpu: draws, pipelines, capture, replay and timing

## Purpose

This part of `crates/victory-gpu` turns what a title list records (pipeline,
heaps, root arguments, vertex and index buffers, draws, dispatches, indirect
executions) into host D3D12 calls on the list's host command list. It also makes
host pipelines from the title's pipeline descriptions, with shaders translated
by `victory-shader`. Around that sit diagnostics: screenshots of presented frames
(`capture.rs`), a recorder and replayer for one frame of host D3D12 work
(`replay/`), and a timeline of threads, waits and card work (`timeline/`).

Callers: the title's list methods in `crates/victory-gpu/src/list.rs` lock the
per-list `draw::State` (`list.rs` `with_state`) and call into it. Device methods
in `crates/victory-gpu/src/device.rs` call `pipeline::create_*`, `root::create`,
`signature::create` and `query::create`. What these files call: host lists,
scratch memory and submission (`crate::host`, see
[victory-gpu-host.md](victory-gpu-host.md)); host copies of title buffers,
textures and descriptor heaps (`crate::shadow`, `crate::texture`,
`crate::resource`, `crate::memory`, see
[victory-gpu-memory.md](victory-gpu-memory.md); `crate::descriptor` is in the
host map).

Background reading: [../architecture.md](../architecture.md),
[../findings.md](../findings.md), [../replay.md](../replay.md) (how to use the
recorder and replayer), [../fork-ray-tracing.md](../fork-ray-tracing.md).

## How it works

### 1. A draw, step by step (`draw.rs`)

`draw::State` holds everything one title list has set: the title pipeline (an
address), the view and sampler `heaps`, `graphics` and `compute` root arguments
as `Root::{Table, Constants, View}`, 32 vertex views, the index view, targets,
viewports, scissors, blend factor, stencil reference, depth bounds; and what
the host list was last given (`host_pipeline`, `host_roots`, `host_heaps`,
`host_targets`).

Setters such as `set_table`, `set_constants` and `set_view` only store. State
that D3D12 keeps across draws goes onto the host list at once: `set_topology`,
`set_targets` (skipped when the same views are already set, `VICTORY_BIND_CACHE`),
`set_viewports`, `set_scissors` and the rest. `State::replay` sets all of this
again on a new host list that continues a title list cut at a memory wait
(`crate::list::cut`, host map).

`State::draw`, `draw_indexed` and `dispatch_mesh` call `prepare_for`. It gets
the host pipeline (`pipeline::host_pipeline`; `None` drops the draw and counts
it in `DROPPED`), checks its kind against the call, calls `bind`, sets the
vertex buffers the input layout reads (`HostPipeline::slots`; each the host copy
from `shadow::find_any`, refreshed with `Shadow::refresh`, bounded by what is
left of it) and the index buffer the same way, and moves targets and depth to
their states (`crate::list::ensure_state`, `list.twin_use`).

`State::bind` is shared by draws, dispatches and indirect executions:

- `early_textures` runs the pass over textures picked by index, if one is due
  (see "Textures and buffers picked by index" below).
- A list without a view heap borrows one (`descriptor::lent_heap`) if the
  pipeline has root views.
- `SetDescriptorHeaps` comes first, then the root signature, then
  `SetPipelineState`. Each is skipped when the host list already has it.
  `list.bind_pipeline` keeps the pipeline alive in the list.
- Then, for each stage of the `HostPipeline`:
  - `tables` gets the descriptor tables ready (below);
  - the stage's user-data block is filled: one dword per entry of the
    translation's `HostShader::user_data`. Each dword is a table's byte offset
    in its heap (`user_value`; or the offset of a table made afresh), a root
    constant, or a value the host works out (`host_word`: memory base, root view
    address halves, view address, view bias, dispatch thread count). It is set
    with `Set*Root32BitConstants`;
  - root CBVs are bound as root descriptors. `root_view` copies a constant
    buffer that is not 256-byte aligned into scratch memory;
  - root SRVs and UAVs are bound as one-view tables of the buffer's host copy
    (`root_srv_table`, `root_uav`), so reads past the copy return zero.
    `VICTORY_ROOT_SRV_DIRECT=1` binds root descriptors instead;
  - our own slots are bound too: `SLOT_MEMORY` (the directory of title memory
    by address, from `memory_ready` -> `crate::memory::ready`), `SLOT_FAULTS`,
    and `SLOT_STRIDES` (`heap_buffers`, for shaders that pick buffers by a
    computed index).

`State::tables(used, indexed, compute)` reads the translation's
`UsedResource` list (`RootType::Table` entries):

- **Plain tables.** A table with no constant buffer, inside the host heap, is
  read through the title's own heap slots. `table_entry` mirrors each slot the
  shader names (`HostHeap::mirror`). `readable` then refreshes buffer ranges,
  declares UAV ranges as GPU-written (`Shadow::gpu_writes`), uploads textures
  the title fills (`texture::upload_from`) and moves textures to the needed
  state.
- **Fresh tables.** A table that holds a constant buffer, or that reaches past a
  smaller host heap, is made afresh in dynamic slots (`HostHeap::dynamic`).
  The host wants constant buffers 256-byte aligned; the console only 16. The
  constant data is copied at submission (`list.defer_constants`). Within one
  list, a table with the same address, slot count and record hash
  (`RecordHash`) is made once (`fresh_tables`). With `VICTORY_READY_ONCE` its
  entries are not walked again while the list's epoch is unchanged. Dynamic
  slots go back in `State::tables_done` after the submission has run
  (`list.rs` `tables_done`), or on drop.
- **Indexed entries.** `root_indexed_views` adds slots a shader picks by a root
  constant: `(byte_offset + (constant << shift)) / 32`.

`State::dispatch` clears the indirect window, applies the ray-tracing builder
rule (`pipeline::is_builder`, `builder_runs`, see 3), calls `bind`, turns thread
counts into groups when the console's "thread dimensions" flag is set
(`THREAD_DIMENSIONS`, `dispatch_groups`, the `USER_DISPATCH_THREADS_X` word),
and calls `Dispatch`. If the shader has an `ambient::Role` and `ambient::on()`,
it then records the stand-in (`State::ambient` -> `ambient::after`) and resets
`host_pipeline` and `host_roots[1]`.

**Textures and buffers picked by index.** Shaders that index the view heap by a
computed value name no table, so `heap_buffers` (bound as `SLOT_STRIDES`) runs
`heap_buffer_pass` once a frame: slots written since the last pass, and a
sixteenth of the rest (`HEAP_ROUND`, `VICTORY_HEAP_SPREAD`), get host slots and
their copies refreshed (handed to the preparing thread with
`list.refreshes_coming` when `host::aside::on()`). `heap_textures` then creates
and uploads textures the title fills, newest slots first, within bytes per
frame (`staging_budget_per_frame`, `VICTORY_TEXTURE_STAGE_MB`) and time per pass
(`VICTORY_TEXTURE_STAGE_MS`), looking at them again after upload
(`RECHECK_FRAMES`). `early_textures` runs the same pass at a frame's first
draw and, with a larger budget, before the title's bake draws (picked by pixel
shader hash, `BAKE_PIXEL_SHADERS`). Slots a draw read through a table leave the
queue (`texture_slot_tabled`). The texture and descriptor side is in the memory
and host maps.

Drawing writes no title memory itself. What shaders store through UAVs and root
UAVs is declared to the shadow layer and copied back after the list has run (see
the memory map). The declaration is skipped for shaders that cannot store
(`Translated::stores`, `VICTORY_WRITE_SKIP`).

### 2. Primitive conversion (`draw.rs`, `primitive.rs`, `pipeline/host.rs`)

`set_topology` maps rectangle lists (17) to triangle lists on the host list; triangle fans, line
loops, quads and quad strips also become triangle lists, logged once, with no vertex reordering.
`HostPipeline::new` sets the topology *type* of rectangle (19) and quad (32) pipelines to triangle,
and gives a rectangle pipeline a geometry stage (`victory_shader::emit::rect_list_gs`) that
completes each rectangle from the vertex shader's outputs for direct, indexed and indirect draws
(`HostPipeline::rects`); if the host refuses it, the pipeline is made without it. Without the stage,
`State::draw` calls `State::rectangles`: per vertex slot, scratch memory for six vertices a
rectangle, filled at submission by `primitive::expand_rects` (`list.defer_host_read`), the fourth
corner from vertex inputs as `v0 + v2 - v1`. Indexed rectangles without the stage are not drawn;
indirect ones without vertex inputs are drawn as their first triangle.

### 3. Pipelines (`pipeline.rs`, `pipeline/*`)

**Creation.** `create_graphics`, `create_compute`, `create_compute_x` and
`create_stream` copy each stage's container into a `Shader`. Shaders are shared
by full BLAKE3 in `SHADERS` and never freed (`kept()` reports their bytes).
`create_stream` walks the subobjects (`subobject` gives sizes; console mesh and
amplification are 23 and 24) and fills a `host::GraphicsState`
(`GraphicsState::read`, formats through `resource::pc_format`). `hand_out` makes
a `make::Pending` keyed by stages, shader hashes and `GraphicsState::describe`,
notes it in the book (`known::note`), starts it early if an earlier run drew
with it (`make::early`), and marks ray-tracing builder stages (a program with a
spilled user-data table, `of_builder`).

The title's root signature is not used for host pipelines. `root.rs` only
serializes and deserializes through the host's `d3d12.dll` (console flags,
bits 24 and up, dropped) and keeps the blob.

**Translation.** `Shader::translated()` is a `OnceLock`, filled on the first
`HostPipeline::new` that needs the shader, on whichever thread makes the
pipeline (usually a worker). `translate` calls `shader_cache::host_shader` (the
on-disk cache, else `victory_shader::emit::translate`), decodes the console
code for `stores`, `stores_by_address`, the group size and the `UsedResource`
list, and with `VICTORY_SHADER_SAVE=<dir>` writes container and DXIL. A shader
that does not translate is logged once; its pipelines' draws are dropped.

**Host pipeline** (`HostPipeline::new`, `pipeline/host.rs`):

- Accepts compute, vertex+pixel, or mesh+pixel; anything else is refused once per reason (`refuse`).
- Builds **our own root signature** from each translation. Per stage: a 32-bit constant block in
  `USER_DATA_SPACE`, root CBVs in `ROOT_CBV_SPACE`, root SRVs and UAVs as one-view tables (except
  our `SLOT_*` ones), static samplers in `SAMPLER_SPACE` (`filter_for_host`, given
  `AnisoFilterWithPointMipSupported`). Flags add `CBV_SRV_UAV_HEAP_DIRECTLY_INDEXED` and
  `SAMPLER_HEAP_DIRECTLY_INDEXED`. `Stage` records which root parameter holds what for `bind`.
- Graphics state: blending is switched off on integer targets (`host_blend`).
  Depth bounds go through a stream with `DEPTH_STENCIL1`
  (`graphics_depth_bounds_pipeline`). Mesh pipelines are streams that name a
  triangle topology, or the library will not load them (`mesh_pipeline`).
- Every pipeline goes through `library::pipeline(name, load, create)`. `name`
  is a BLAKE3 of kind, root blob, DXIL and state text.

**Workers and budgets** (`pipeline/make.rs`). `make::ready` is called by
`pipeline::host_pipeline` at a draw. It notes the key as used
(`library::note_used`); if the pipeline is not made it queues it as
`REQUESTED` (`start`) and waits on the condvar `DONE`, out of a budget per
title frame shared by all threads (`VICTORY_PIPELINE_WAIT_MS`, 500 ms;
`VICTORY_PIPELINE_WAIT_SHARED`). Past the budget the draw is dropped; the
pipeline is still made. `Queue` pops in this order: `screen` (requested passes
over the screen, `GraphicsState::screen_pass`, with their own budget
`VICTORY_SCREEN_WAIT_MS`, 10 s, and two extra workers that take only them,
`SCREEN_WORKERS`), `requested`, `early`, then `slow` (early ones that took 1 s
or more in some run, `library::slow`; at most a quarter of the workers at once,
`slow_limit`, `VICTORY_PIPELINE_SLOW`).

There are two thirds of the logical processors as workers, 2 to 16
(`VICTORY_PIPELINE_WORKERS`). They run at lowest priority
(`VICTORY_PIPELINE_LOWEST`) and are named `victory-pipelines-<n>`. After each
pipeline a worker calls `library::save(false)`, which saves at most every 20 s.
`make::now` makes a pipeline on the calling thread; the builder rule uses it.

**On-disk library** (`pipeline/library.rs`). Under the pipelines folder:
`host-pipelines.bin` (an `ID3D12PipelineLibrary1`, mapped, `Blob::Mapped`,
`VICTORY_LIBRARY_MAP`, its handle held open, `VICTORY_LIBRARY_HANDLE`; `trim`
lets its pages go after a quiet spell), `early-pipelines.txt` (keys drawn with
in this or an earlier run) and `slow-pipelines.txt`. `pipeline()` loads by name under `same_name` (one of 64 locks). If the load
fails it creates the pipeline outside any lock and stores it under a read lock
of `saving`. `save` takes the write lock, serialises into `.tmp` through a
mapping, and renames it over the library (or to `host-pipelines.next` for the
next start).

**Known pipelines and `victory warm`** (`pipeline/known.rs`). With
`VICTORY_PIPELINE_BOOK=1`, every created pipeline is appended to
`known-shaders.bin` and `known-pipelines.bin` (a thread `victory-known` flushes
every 2 s; a torn end is cut on open). `apps/victory/src/warm.rs` (through
`victory_hle::known_pipelines`) uses `keys`, `fingerprint` (translator build
plus driver), `use_translations` (the shader cache only) and `make`, which
rebuilds the `Shader`s and `GraphicsState` (`GraphicsState::put`/`get`) and
calls `HostPipeline::new`. The driver's own cache keeps the results. Each child
process makes up to 40; a crash or hang goes to `warm-bad.txt`.

**Shader cache** (`shader_cache.rs`). It goes through `victory_pipeline::Cache`
with converter id `shader.dxil`; the input is `build()` then the container.
`build()` hashes `VICTORY_TRANSLATOR` (BLAKE3 of the translator's sources, from
`build.rs`), the loop limit, group sync, `shader_opt`, a fixed list of
`VICTORY_*` translator switches, and `emit::host_settings()` (after
`probe_view_limit`). Entries are our own `VSH1` encoding of `HostShader` (`encode`/`decode`). An
entry that does not decode is translated again. `precompile` serves
`victory shaders`.

**Ray-tracing builder rule** (`pipeline.rs` `Builder`). Builder dispatches run
only with `VICTORY_BVH_BUILDER=1` and while every builder pipeline created so
far has a host pipeline. `builder_runs` calls `make::now` on each the first
time. A stage that cannot be bound calls `builder_rests`: no stage runs this
frame and the next. `builder_quarantined` (`VICTORY_BVH_QUARANTINE=1`) keeps
their stores out of title memory.

### 4. Indirect and multi-draw execution (`draw.rs`, `indirect.rs`, `indirect/batch.rs`)

`list.rs` `execute_indirect` takes `signature::layout`. It runs only
one-argument signatures: draw, indexed, dispatch, mesh, and the console's type
105 as a dispatch. It calls `State::execute_indirect((kind, stride, max),
address, count)`.

Arguments are never read by the GPU from the title's buffer as they are: our compute shaders (HLSL,
`cs_5_1`, `D3DCompile`, root signature inside) clamp them into a device-local scratch buffer.

1. **Batches** (default, `VICTORY_INDIRECT_BATCH=1`), for one record without a count buffer (all of
   the title's). `State::batched` asks the open `indirect::batch::Table` for a place
   (`Table::place`); without one, `indirect::open_batch` records a pass of `JOBS` (256) threads over
   a job table in upload memory that the CPU goes on filling while the list is recorded. Up to
   `SOURCES` (4) argument host copies can be read (`ARGUMENT_COPIES`); a record asked for again
   reuses its job (`TIMES`, at most 31 charges). The batch ends where arguments may change: a
   dispatch, a draw whose shaders store (`State::drew`), a copy (`State::copied`), another host list
   (`list.first_use(BATCH_USE)`). With `=2`, dispatches end it only at the title's next barrier,
   sync or draw (`State::synced`).
2. **Windows** (`VICTORY_INDIRECT_WINDOW=1`, off): records walked at a fixed step are clamped ahead,
   16 then doubling up to `WINDOW_MOST` (64), by `indirect::bound_ahead`.
3. **Single clamp** (`indirect::bound`) for the rest (count buffers, `max > 1`).

The clamp (`indirect.rs` `SOURCE`, `batch.rs` `SOURCE`) bounds draws below 2^23 vertices an
instance and 2^26 vertex instances; dispatches to 65,535 groups a dimension but x up to `WIDEST`
(2^20 on AMD with `VICTORY_INDIRECT_WIDE`, else 65,535) and 2^20 groups in all; mesh dispatches to
one dimension, over 65,535 groups taken as garbage and run empty. It charges a **per-list budget**
of 2^27 vertex instances or threads (a group 64, a mesh group 128), once per 64-thread group in lane
order with one compare-exchange loop. A record that does not fit runs empty and, with
`VICTORY_INDIRECT_REST` (on), takes the rest, so later records run empty too. The budget buffer's
next words collect refusals and the largest cost for the frame line (`list.budget()`, host map).

The scratch buffer goes `before -> UNORDERED_ACCESS -> INDIRECT_ARGUMENT` around each clamp, with
UAV barriers, and stays readable as arguments until the next clamp (`list.indirect_state`,
`list.indirect_readable`): executions need no barrier of their own. `host_signature(kind, stride)`
caches one-argument host command signatures (no root signature). After any clamp `State` forgets
`host_pipeline` and `host_roots[1]`.

### 5. The indirect-light stand-in (`ambient.rs`)

See [../fork-ray-tracing.md](../fork-ray-tracing.md), section "Without rays".

`ambient::role(container)` tags three compute programs by name when their
`Shader` is made (`Role::Radiance`, `Resolve`, `Air`). After a dispatch of one,
`State::ambient` finds the image and depth views at fixed (root parameter,
table dword) places (`LIGHT`, `DEPTH`, `RADIANCE`, `AIR`), checks formats and
sizes, and calls `ambient::after`. That records our passes from one HLSL source
(`SOURCE`, `crate::host::compile`): `keep` and `mean` after `Radiance` (sets
`KEPT`), `place`, `gather`, `raise` after `Resolve`, `clear_air` after `Air`,
using two buffers of ours, `cells` (64x36) and `points` (160x90). `on()` is
false while `builder_runs()` and unless `VICTORY_AMBIENT` is a share or
`constant:<level>`.

### 6. Frame capture (screenshots) (`capture.rs`, diagnostics)

This is not the replay recorder. `VICTORY_CAPTURE=<folder>` saves presented
frames as `frame-<n>.png` at half size (`wanted`: powers of two, listed frames,
and world-relative frames with `VICTORY_CAPTURE_WORLD`). The present path in
`host.rs` calls `read_back`, `save_read_back` and `probe`/`black`. `probe`
checks a 4x4 grid of pixels to detect black frames; `recent_levels` keeps the
last twelve. `op` writes `frame-<n>-ops.txt`, one line per operation, for listed
frames. Callers include `draw.rs` (`list_dropped_roots`, builder dispatches)
and the host layer; `State::op_text` is the text a list keeps through
`HostList::note_op` for the device-removed report. `dump_targets` (`VICTORY_DUMP_TARGETS`)
saves every target such a frame wrote (called from `queue.rs`). `png` is shared
with the replay recorder.

### 7. Record and replay at the code level (`replay/`, diagnostics)

User-facing description: [../replay.md](../replay.md). Code map:

**Hooking** (`replay/hook.rs`). `hook::install(object, state, patch)` gives
**one COM object** its own copy of its method table (`Shim`: state, original
table, 4 words before it, 512 slots). Other objects of the class are untouched.
`orig::<Vtbl>` calls the real method, `state::<T>` returns the object's
recorder state, and `again` fixes the pointer after a method (list `Reset`)
writes it anew. Methods we cannot record get `noted::<AT>`, a naked-asm stub
that counts the call in `COUNTS` and jumps on. The counts are listed in
`notes.txt` (`not_recorded_calls`).

**Recorder** (`replay/record.rs`):

- `host.rs` calls `record::from_env(device)` right after the device is made.
  `Settings::from_env` reads `VICTORY_RECORD*`, `start` creates the
  process-wide `Recorder` (`RECORDER`, one capture per process) in
  `<folder>/capture-<n>`, and hooks every device interface (`device_table`).
- Device hooks record creations into the recorder's maps: resources, buffer
  addresses, descriptor heaps, roots, pipelines (`pso::graphics`, `compute`,
  `stream` into `wire::Part`s), signatures, query heaps, names. Shader code goes
  to `shaders.bin` once by hash (`Codes`). Descriptor writes and copies are
  mirrored into `record::Heap` slots (`wire::Slot`).
- Lists created through the device get `list_table`. `list_reset` sets
  `whole = armed && direct`. `note` appends a `wire::Cmd` only while armed.
  `list_close` saves the bytes and the state changes (`states::changes`).
- The queue hook `execute` sends direct-queue submissions to `snap::submit`;
  other queues go to `snap::other_queue` and are noted as missing.
- Calls our own code makes are not recorded: `Ours` or `unrecorded()` sets a
  thread-local count (used by the panel, `host/panel.rs`).
- `record::name` (from `crate::host::name`) labels objects. The replayer finds
  our own passes by these labels: `play::CLAMP` ("victory: argument clamp")
  and `play::COMPARE`.

**Snapshot** (`replay/snap.rs`). `snap::Phase` steps through `Stage::Waiting ->
Armed -> Capturing -> Done`. It advances on each `record::present` (called from
`host.rs` before the swap chain's present).

- **Armed.** It waits at least 2 presents for a frame whose lists were all
  recorded whole, at most `PATIENCE` (100), then builds a `Capture` (a
  `gpu::Worker` per queue).
- **First submission** (`Capture::submit` -> `start`). It drains the card,
  takes a reference to every live resource (`held`), finds idle aliased placed
  resources (`find_idle`), copies the card's memory into `contents.bin`
  through 4 read-back buffers of 256 MB in pieces up to 32 MB (`snapshot`), and
  records the runs of every shader-visible descriptor heap (`runs_of`).
- **Each later submission.** The card is drained; descriptor slots written
  since go into `Submit::views`; `Reads::note` finds what the lists read of
  CPU-visible buffers; `changed` compares those pages with the kept copy (4 KiB
  pages, 1 MB chunks across threads) and writes `Delta`s to `deltas.bin`.
- **Present.** `Capture::present` reads back the presented texture into
  `present.bin` and `frame-<n>.png`.
- **Finish.** `finish` writes `frame.bin` (`wire::Frame`) and `notes.txt`.
  On an error, `stop` writes `failed.txt`.

**Wire format** (`replay/wire.rs`). Plain little-endian, `MAGIC` `VICFRAME`,
`VERSION` 1. `Cmd` has 33 numbered variants. `Slot` is a descriptor by content
(kind, resource id, desc bytes). `Va` is a GPU address as (resource, offset).
`Frame` holds the adapter, notes, heaps, resources (with start states), roots,
codes, pipelines, signatures, descriptor heaps, query heaps, lists, submits,
contents and presents.

**Replayer** (`replay/play.rs`). `apps/victory/src/replay.rs` calls
`play::replay(folder, &Options)` or the `Player` methods. `Player::load`: `find`
(the newest `capture-<n>`), `make_device` (debug layer, GPU-based validation
with `VICTORY_REPLAY_GBV`, breadcrumbs), list parsing, `make_resources`
(`--buffers`, `--only-buffers`), `make_pipelines` (`pso::make`, or DXIL from
`--shaders`), `make_descriptors` (heap versions per submission whose views
change), `load_contents`, `make_masters` (copies to restore from).

`run` / `replay` -> `prepare` -> `edited` -> `build` -> `once` x loops -> `finish`:

- **`edited`** applies the `replay/edit.rs` experiments to each list's
  commands: `skip_pipelines`, `barriers_only`, `skip_clamps` (with
  `clamp_results` from a first run), `batch_clamps` (the same algorithm as
  `VICTORY_INDIRECT_BATCH`, with `crate::indirect::batch_for`),
  `no_uav_barriers` and `merge_barriers`.
- **`build`** records, per submission:
  - a delta list that copies what the CPU wrote from one upload buffer;
  - unless `--no-merge` (`Options::merge`), a pass `play/merge.rs` `Merger` that keeps
    words the card has written in this run;
  - then each list through `translate` -> `command`.
- **`once`** puts the frame's resources back (`restore`), sends the lists
  (`send`; `--one-submit`, `--wait-lists`, `--pace`) and reads the
  timestamps.
- **`finish`** adds the per-command detail (`detail`), the merge notes, the
  picture (`picture`: PSNR against the recorded present, written to
  `replay.png`) and debug messages.

Investigation modes, each an `impl Player` in its own file: `play/diverge.rs`
(`diverge`, `diverge_picture`, `diverge_draws`, `watch`, `slots`,
`check_written`), `play/pixel.rs` (`pixel`), `play/look.rs` (`flow`, no device;
`images`), `play/upcopies.rs` (`upcopies`).

`replay/states.rs` follows texture states through barriers, promotions and
decay at the end of a submission. `replay/gpu.rs` has `Worker` (a queue, a
fence, `run`, `idle`) and resource, footprint and PSNR helpers.

### 8. Per-pass timing

**In a replay** (`play.rs` `build`, `translate`, `detail`):

- `open` and `close` put an `EndQuery` timestamp at the start and end of every
  list.
- With `--timing detailed`, `translate` also puts one after every command that
  works (`Cmd::works`) or is a barrier. It labels each stamp with a category
  string ("argument clamps (ours)", "hash and compare passes (ours)",
  "indirect mesh draws", the copy kinds from `copy_kind`, and so on) and with
  the bound pipeline id, or `COPY_TO | texture`.
- `once` turns stamps into `Report::frame_ms`, `busy_ms`, `delta_ms` and
  `submits` using `GetTimestampFrequency`.
- `detail` sums each loop by category and by pipeline. It takes the median
  across loops, leaving out the first loop when there are more than two, and
  keeps the top `o.top` pipelines. `Report::text` prints it.

**In a live run** (`timeline/`, diagnostics):

- `VICTORY_TIMELINE` (`record.rs` `asked`, `presented`) starts a recording two
  presents after the given seconds and writes it from a `victory-timeline`
  thread after the given frames.
- Events go into one preallocated buffer of `EVENTS` (2^20) slots, one atomic
  add per write; events past it are counted as lost. Kinds: `Span`
  (`timeline::span`, used throughout the crate), `Calls` (`called`, list calls
  joined when under 10 uss apart), `Wait`/`Wake` (`hooks.rs` wrappers; the
  title's imports are bound through `hooks::import`, our own import table is
  patched by `wrap_own`), `Kernel`, `Sent`/`Told`/`Poll` (from `queue.rs`,
  `fence.rs`, `host.rs`), and `Sample` (a `victory-sample` thread suspends the
  title's submitting threads every quarter millisecond).
- The card's start and end of each submission come from host timestamps
  (`crate::host::timeline_stamps`, host map).
- `report.rs` `Trace::chain` walks back from each present: through waits to the
  thread that woke them, and through the card's submissions to the sender.
  `summary` writes the `.txt`; `chrome` writes the JSON trace.

The per-frame numbers on the live frame line (`host/stamps.rs`,
`host/spans.rs`) belong to the host map.

## Files

| Path | What it does | Key items |
|---|---|---|
| `crates/victory-gpu/src/draw.rs` | Per-list draw state. It brings the host list up to date for each draw, dispatch, mesh dispatch and indirect execution. Also the passes over buffers and textures picked by index. | `State`, `Root`, `State::{bind, prepare_for, tables, draw, draw_indexed, dispatch, dispatch_mesh, execute_indirect, batched, replay, tables_done, heap_buffers, heap_textures, early_textures, ambient}`, `readable`, `root_view`, `host_signature`, `heap_buffer_pass`, `staging_budget_per_frame` |
| `crates/victory-gpu/src/indirect.rs` | The indirect-argument clamp (HLSL), its budget and scratch-buffer states. Its `tests` module also holds WARP and card measurements that are not about indirect execution: card compare, raw loads, descriptors under readers, new blocks, and `bare_cost`. | `SOURCE`, `bound`, `bound_ahead`, `open_batch`, `batch_for`, `widest`, `rest`, `amd`, `record_clamp_as`, `validate_layout` |
| `crates/victory-gpu/src/indirect/batch.rs` | One clamp pass for all executions up to the next point where arguments may change. It reads a job table the CPU fills while recording. | `Table::{new, place}`, `record`, `JOBS`, `SOURCES`, `TIMES`, `SLOT`, `SOURCE` |
| `crates/victory-gpu/src/indirect/bench.rs` | Ignored benchmark: 3,200 one-record indirect draws, the old barrier scheme against the current one. | `bench_indirect_draw_barriers`, `OLD_SOURCE` |
| `crates/victory-gpu/src/pipeline.rs` | Title pipeline objects, shared shader containers, translation on first use, the ray-tracing builder rule, the skip list, device entry points. | `Shader`, `Translated`, `translate`, `hand_out`, `create_graphics`/`create_compute`/`create_compute_x`/`create_stream`, `subobject`, `host_pipeline`, `Builder`, `builder_runs`, `is_builder`, `stores`, `stores_by_address`, `is_skipped` |
| `crates/victory-gpu/src/pipeline/host.rs` | Host pipelines: our root signature from the translation, the graphics state, rectangle geometry stage, depth-bounds and mesh streams. | `GraphicsState::{read, describe, screen_pass}`, `HostPipeline::new`, `Stage`, `host_blend`, `root_signature`, `mesh_pipeline`, `graphics_depth_bounds_pipeline`, `bounded_root_srvs` |
| `crates/victory-gpu/src/pipeline/library.rs` | Host pipeline library on disk (mapped file), early list, slow list, saves. | `pipeline`, `name`, `save`, `trim`, `wanted_early`, `note_used`, `slow`, `note_time`, `same_name`, `removal_note` |
| `crates/victory-gpu/src/pipeline/make.rs` | Worker threads that make host pipelines, the per-frame wait budget, queue priorities. | `Pending`, `ready`, `now`, `early`, `wait`, `Queue`, `workers`, `worker_count`, `slow_limit`, `screen_budget` |
| `crates/victory-gpu/src/pipeline/known.rs` | Book of every pipeline the title created (`VICTORY_PIPELINE_BOOK`), and making them ahead for `victory warm`. | `note`, `flush`, `keys`, `count`, `fingerprint`, `use_translations`, `make`, `GraphicsState::{put, get}` |
| `crates/victory-gpu/src/root.rs` | Title root signatures: serializing through the host `d3d12.dll`, console flags dropped. The blob is kept but not used for host pipelines. | `serialize`, `serialize_versioned`, `create_deserializer`, `create`, `CONSOLE_FLAGS` |
| `crates/victory-gpu/src/signature.rs` | Command signatures: stride and argument list kept. `layout` feeds `list.rs` `execute_indirect`. | `create`, `layout` |
| `crates/victory-gpu/src/sampler.rs` | Console sampler words (`CreateSamplerX`) to a host `D3D12_SAMPLER_DESC`. Called from `descriptor/host.rs`. | `console`, `Fields::host`, `address_mode`, `finite_lod` |
| `crates/victory-gpu/src/primitive.rs` | CPU expansion of rectangle lists into triangles (fourth corner `v0 + v2 - v1`), by input format. | `expand_rects`, `components`, `Kind`, `half_to_f32`, `f32_to_half` |
| `crates/victory-gpu/src/shader_cache.rs` | On-disk cache of translations, keyed by container and translator build and settings. | `use_shader_cache`, `host_shader`, `precompile`, `translation`, `build`, `encode`, `decode`, `counts` |
| `crates/victory-gpu/src/ambient.rs` | Stand-in passes for the traced indirect light (`VICTORY_AMBIENT`). | `Role`, `role`, `on`, `after`, `Views`, `Pass`, `SOURCE`, `LIGHT`/`DEPTH`/`RADIANCE`/`AIR` |
| `crates/victory-gpu/src/query.rs` | Query heap objects; the description only. `list.rs` answers queries with plausible values. | `create`, `Desc` |
| `crates/victory-gpu/src/capture.rs` | Diagnostics screenshots of presented frames, ops listings, target dumps, black-frame probe. | `folder`, `wanted`, `listed`, `op`, `read_back`, `save_read_back`, `probe`, `black`, `dump_targets`, `note_target`, `png` |
| `crates/victory-gpu/src/replay/mod.rs` | Module list of the recorder and replayer. | (modules) |
| `crates/victory-gpu/src/replay/hook.rs` | Per-object method-table shims. | `install`, `orig`, `state`, `again`, `ours`, `remove`, `noted`, `count`, `method!` |
| `crates/victory-gpu/src/replay/record.rs` | The recorder: device, queue, list and library hooks; objects, descriptors and states kept; commands encoded while armed. | `Settings`, `At`, `Recorder`, `start`, `from_env`, `present`, `finished`, `unrecorded`, `ListState`, `execute`, `note`, `list_reset`, `list_close` |
| `crates/victory-gpu/src/replay/snap.rs` | One frame written down: card memory at the start, CPU changes per submission, presents, `frame.bin`. | `Phase`, `Stage`, `submit`, `present`, `Capture::{start, snapshot, changed, submit, present, finish}`, `Reads` |
| `crates/victory-gpu/src/replay/wire.rs` | Capture file format. | `Frame`, `Cmd`, `Slot`, `Va`, `Barrier`, `Resource`, `Submit`, `Delta`, `Content`, `Present`, `Wire`, `VERSION`, `MAGIC` |
| `crates/victory-gpu/src/replay/states.rs` | Texture states as lists left them. | `States`, `Change`, `changes`, `decay` |
| `crates/victory-gpu/src/replay/pso.rs` | Pipeline descriptions as stream parts, and made again from them. | `graphics`, `compute`, `stream`, `make`, `Keep` |
| `crates/victory-gpu/src/replay/gpu.rs` | Helpers shared by the recorder and the replayer. | `Worker`, `buffer`, `footprints`, `transition`, `to_desc`, `seen`, `Seen`, `rgb`, `psnr`, `adapter` |
| `crates/victory-gpu/src/replay/edit.rs` | Experiments on command lists. | `skip_pipelines`, `no_uav_barriers`, `barriers_only`, `merge_barriers`, `find_clamps`, `keep_clamped`, `skip_clamps`, `batch_clamps`, `BATCH*`, `RESULTS` |
| `crates/victory-gpu/src/replay/play.rs` | The replayer: objects made again, contents loaded, runs, timing, report, picture. | `Options`, `Report`, `Player::{load, run, prepare, edited, build, translate, command, send, once, finish, detail, picture}`, `replay`, `find`, `listing`, `dump`, `CLAMP`, `COMPARE` |
| `crates/victory-gpu/src/replay/play/diverge.rs` | Where identical runs differ: by list, by picture, by draw; `--watch`, `--slots`, `--check-written`. | `Search`, `Player::{diverge, diverge_picture, diverge_draws, watch, slots, check_written, hashes_now}` |
| `crates/victory-gpu/src/replay/play/pixel.rs` | `--pixel`: which commands changed a texel. | `Pixels`, `Player::{pixel, probed, take, sampled}`, `part_of` |
| `crates/victory-gpu/src/replay/play/look.rs` | `--flow` (bindings without a device) and `--images`. | `flow`, `Images`, `Player::images` |
| `crates/victory-gpu/src/replay/play/upcopies.rs` | `--upcopies`: stale words in CPU-to-card copies, undeclared shader writes. | `Player::upcopies`, `ups`, `declared` |
| `crates/victory-gpu/src/replay/play/merge.rs` | CPU copies merged without overwriting words the card wrote in this run. | `Merger`, `Player::{merged_copy, merge_all, set_merging, merge_notes}`, `SOURCE` |
| `crates/victory-gpu/src/timeline.rs` | Timeline module front; no-op `Span` without diagnostics. | `span`, `Span`, re-exports `called`, `card_wait`, `sent`, `told`, `polled`, `presented`, `title_thread`, `import` |
| `crates/victory-gpu/src/timeline/record.rs` | Lock-free event buffer, spans, waits, samples, start/stop per `VICTORY_TIMELINE`. | `Kind`, `Event`, `put`, `Span`, `called`, `card_wait`, `presented`, `sample`, `asked`, `begin`, `end` |
| `crates/victory-gpu/src/timeline/report.rs` | Sorting events into chains per present; text summary and Chrome trace. | `Trace::{new, chain, card}`, `summary`, `chrome`, `write`, `Threads`, `Names` |
| `crates/victory-gpu/src/timeline/hooks.rs` | Wrappers of wait and wake functions for the title's imports and our own import table. | `import`, `wrap_own`, `wait`, `lock_of`, `released` |
| `crates/victory-gpu/tests/replay.rs` | Records and replays a small D3D12 program on the PC's adapter and compares every byte (skipped without one). | `a_frame_is_recorded_and_replayed`, `Scene`, `check` |
| `crates/victory-gpu/tests/replay_big.rs` | Ignored: recorder and replayer at a title's frame size. | `a_frame_of_a_titles_size`, `Big` (`BIG_TEXTURES`, `BIG_CARD`, `BIG_FOLDER`) |

`crates/victory-gpu/Cargo.toml` (in no map's list) defines the `diagnostics`
feature. `capture`, `replay` and the real `timeline` exist only with it.

## Switches

Defaults are as read in the code. "on" means unset is on and `=0` turns it off.

| Name | Default | What it changes | Read in |
|---|---|---|---|
| `VICTORY_BIND_CACHE` | on | `=0` sets render targets again even when unchanged | `draw.rs` |
| `VICTORY_WRITE_SKIP` | on | UAV ranges of shaders that cannot store are not declared written | `draw.rs` |
| `VICTORY_READY_ONCE` | on (`toggled!`, A/B bit in diagnostics) | fresh tables not walked again within a list epoch | `draw.rs` |
| `VICTORY_ROOT_SRV_DIRECT` | unset | `=1` (any value): root SRVs and UAVs as root descriptors, not one-view tables | `pipeline/host.rs` |
| `VICTORY_ROOT_VIEW_MB`, `VICTORY_ROOT_UAV_MB` | no limit | caps the range of root views and root UAVs | `draw.rs` |
| `VICTORY_THREAD_DIMENSIONS` | on | dispatch counts with the thread-dimension flag are thread counts | `draw.rs` |
| `VICTORY_INDIRECT_BATCH` | 1 | 0: no batches; 2: dispatches end a batch only at the title's wait | `draw.rs` |
| `VICTORY_INDIRECT_WINDOW` | off | `=1`: windows of records clamped ahead | `draw.rs` |
| `VICTORY_INDIRECT_WIDE` | on | AMD only: indirect dispatches up to 2^20 groups in x | `indirect.rs` |
| `VICTORY_INDIRECT_REST` | on | a record over budget takes the rest of it | `indirect.rs` |
| `VICTORY_MESH_TEST` | unset | `skip`, `scissor`, `keep:<hex>[:<hex>]` experiments on mesh draws | `draw.rs` |
| `VICTORY_SCISSOR_TEST` | unset | `full`: every scissor widened to 16384 | `draw.rs` |
| `VICTORY_EARLY_TEXTURES_MS`, `VICTORY_BAKE_TEXTURES_MS` | 12, 1500 | time budgets of the pass over textures picked by index: early in the frame, and before bake draws | `draw.rs` |
| `VICTORY_BAKE_EXACT`, `VICTORY_RECHECK_SOON` | on | bake draws see title memory as it is now; a changed texture is looked at again next frame | `draw.rs` |
| `VICTORY_HEAP_TEXTURES`, `VICTORY_HEAP_RECHECK`, `VICTORY_HEAP_SPREAD`, `VICTORY_FULL_CARD_PASS`, `VICTORY_REUPLOAD_WHEN_FULL`, `VICTORY_TEXTURE_CATCHUP` | on | parts of the passes over heap textures and buffers | `draw.rs` |
| `VICTORY_TEXTURE_STAGE_MB`, `VICTORY_TEXTURE_STAGE_MS` | from VRAM (`staging_budget_per_frame`), 250 | heap-texture bytes and time per frame | `draw.rs` |
| `VICTORY_UNWATCHED_FRAMES` | 1 | frames between refreshes of unwatched host copies in the heap pass | `draw.rs` |
| `VICTORY_CARD_ACCOUNTING` | off | `=1`: card accounting lines | `draw.rs` |
| `VICTORY_LIST_SLOTS` | unset | pixel-shader hash prefixes whose draws get their slots listed (diagnostics) | `draw.rs` |
| `VICTORY_SKIP_PIPELINES` | unset | 16-digit shader hashes whose draws are skipped (diagnostics) | `pipeline.rs` |
| `VICTORY_SHADER_SAVE` | unset | folder for each container and its DXIL | `pipeline.rs` |
| `VICTORY_BVH_BUILDER`, `VICTORY_BVH_QUARANTINE` | off | ray-tracing builder stages run; their stores stay on the host | `pipeline.rs` |
| `VICTORY_PIPELINE_WAIT_MS` | 500 | wait per title frame for pipelines being made | `pipeline/make.rs` |
| `VICTORY_PIPELINE_WAIT_SHARED` | on | that budget shared by all threads | `pipeline/make.rs` |
| `VICTORY_SCREEN_WAIT_MS` | 10000 | budget for passes over the screen; 0: no separate lane | `pipeline/make.rs` |
| `VICTORY_PIPELINE_WORKERS` | 2/3 of CPUs, 2 to 16 | worker count (1 to 64) | `pipeline/make.rs` |
| `VICTORY_PIPELINE_SLOW` | on | `=0`: no quarter-of-workers limit for slow early pipelines | `pipeline/make.rs` |
| `VICTORY_PIPELINE_LOWEST` | on | workers at lowest priority, not below normal | `pipeline/make.rs` |
| `VICTORY_EARLY_ALL` | off | `=1`: every pipeline started at creation | `pipeline/library.rs` |
| `VICTORY_LIBRARY_MAP`, `VICTORY_LIBRARY_HANDLE`, `VICTORY_LIBRARY_TRIM` | on | library file mapped, handle held, pages trimmed | `pipeline/library.rs` |
| `VICTORY_LIBRARY_LOCK` | off | `=1`: one library call at a time | `pipeline/library.rs` |
| `VICTORY_PIPELINE_BOOK` | off | `=1`: every created pipeline kept for `victory warm` | `pipeline/known.rs` |
| `VICTORY_SHADER_LOOP_LIMIT`, `VICTORY_WAVE_*`, `VICTORY_F16_NEAREST`, `VICTORY_HELPER_OWN`, `VICTORY_FIRSTLANE_OWN`, `VICTORY_FIRST_HALF`, `VICTORY_STRICT_JOINS`, `VICTORY_MESH_OUTPUT_STRICT`, `VICTORY_BVH_FENCES`, `VICTORY_BVH_BOUNDS` | (translator's) | only hashed into the shader cache key here; read by `victory-shader` | `shader_cache.rs` |
| `VICTORY_AMBIENT`, `VICTORY_AMBIENT_SHADE`, `VICTORY_AMBIENT_AIR` | off, `1,0.6`, off | indirect-light stand-in | `ambient.rs` |
| `VICTORY_CAPTURE`, `VICTORY_CAPTURE_FRAMES`, `VICTORY_CAPTURE_WORLD`, `VICTORY_CAPTURE_WORLD_ALSO`, `VICTORY_DUMP_TARGETS` | unset | screenshots, ops listings, target dumps | `capture.rs` |
| `VICTORY_RECORD`, `VICTORY_RECORD_AT`, `VICTORY_RECORD_FRAMES`, `VICTORY_RECORD_ALWAYS` | unset, trigger file, 1, off | frame recorder ([../replay.md](../replay.md)) | `replay/record.rs` |
| `VICTORY_REPLAY_GBV` | unset | GPU-based validation with `--debug-layer` | `replay/play.rs` |
| `VICTORY_REPLAY_PEEK` | unset | `diverge_draws` prints what draws read even with one outcome | `replay/play/diverge.rs` |
| `VICTORY_TIMELINE`, `VICTORY_TIMELINE_AT`, `VICTORY_TIMELINE_FRAMES` | unset, 60 s, 3 (1 to 60) | the timeline | `timeline/record.rs` |
| `VICTORY_TEST_D3D_DEBUG` | unset | debug layer in `crates/victory-gpu/tests/replay.rs` | `crates/victory-gpu/tests/replay.rs` |
| `VICTORY_BENCH_*`, `VICTORY_BLOCK_WAY`, `VICTORY_PC_DXC` | unset | test and benchmark knobs | `indirect.rs`, `pipeline/host.rs` tests |

These appear in comments here but are read elsewhere: `VICTORY_INDIRECT_CARD_BUDGET`,
`VICTORY_VIEW_EXTENT`, `VICTORY_BARRIER_TEST` (`copystat.rs`),
`VICTORY_D3D12_DRED`, `VICTORY_D3D12_DEBUG`.

## Traps

- **Bind order.** `SetDescriptorHeaps` must precede the root signature (it indexes heaps directly);
  `State::bind` clears `host_roots` when it sets heaps. Any pass of ours that binds its own compute
  root signature or pipeline must clear `host_pipeline` and `host_roots[1]` after it (the clamp,
  batches and `ambient` do), or the title's next dispatch runs under our signature.
- **Two topology numberings.** `draw.rs` `RECTANGLES = 17` is a primitive topology;
  `pipeline/host.rs` `RECTANGLES = 19`, `QUADS = 32` are topology types.
- **Dynamic descriptor slots** go back after the submission has run (`tables_done`), not at reset:
  a list submitted again without a reset reads slots others may own (logged once). No free slots:
  `tables` returns `u32::MAX` and `bind` drops the draw.
- **The title's root signature is ignored** for host pipelines. A change to how translated shaders
  read user data touches `HostShader::user_data` (`victory-shader`), `State::host_word` /
  `user_value` and `HostPipeline::new` together. Host-side settings that change a translation must
  go into `emit::host_settings` or the `build()` switch list, or old cache entries are served.
- **`shader_cache.rs`**: a field added to `HostShader` must be added to `encode` and `decode`.
- **Library names** cover kind, root blob, code and state ("mesh triangles" keeps older mesh entries
  from being looked up). Do not close the mapped library's handle before a save renames over it:
  runs lost the device that way (module doc of `library.rs`).
- **Builder stages run all or none**; dropping a builder dispatch must go through `builder_rests`.
- **Indirect scratch** stays in `INDIRECT_ARGUMENT` after a clamp and `HostList` tracks it
  (`indirect_state`): do not transition it elsewhere. A batch's job table is written after its pass
  was recorded, valid only because the list runs after it is closed (`Table::new` safety comment).
- **One garbage record can empty every later draw of a list** (`VICTORY_INDIRECT_REST`);
  docs/findings.md suspects this for outdoor flicker.
- **`ambient.rs`** finds its programs by name and its views at fixed table places: another title
  build silently gets no stand-in ("no stand-in ...: other tables", logged once).
- **Replay finds our passes by debug name** (`play::CLAMP`, `play::COMPARE`, through
  `crate::host::name` -> `record::name`). Renaming breaks `--skip-clamps`, `--skip-hash-passes` and
  the timing categories.
- **`wire.rs` `Cmd` numbers are the format.** A new command needs a number, `LAST_CMD` and
  `every_command()` in tests, a recorder hook, `states::changes`, `Player::command`, usually
  `VERSION`.
- **Recorder hooks are per object**; `list_reset` must call `hook::again`. Our own D3D12 calls during
  a capture must run under `Ours` / `unrecorded()`, or they land in the title's lists.
- **Pipeline waits block title threads** up to the frame budget; past it draws are dropped, so a
  pass that runs once and is not ready is lost (module doc of `make.rs`).
- **Stale comments**: `pipeline.rs`'s module doc says mesh stages get no host pipeline
  (`HostPipeline::new` makes them); `signature.rs` says "kept for translating it later" (`layout` is
  used); `query.rs` says "Nothing runs on a GPU yet".

## Where to start for common changes

- Draw dropped or wrong: `State::prepare_for` / `State::bind` in `crates/victory-gpu/src/draw.rs`;
  diagnostics give reasons through `pipeline::note_why` / `take_drops`.
- New root binding kind from the translator: `HostPipeline::new` (`pipeline/host.rs`), the matching
  loop in `State::bind`, `State::host_word`.
- New console topology: `State::set_topology`, `HostPipeline::new` (`PrimitiveTopologyType`),
  `primitive.rs`.
- Indirect limits or budget: both HLSL sources (`indirect.rs`, `indirect/batch.rs`); WARP tests in
  `indirect.rs` `tests`.
- Pipeline stalls: `pipeline/make.rs` (budgets, queue), `pipeline/library.rs` (early and slow lists),
  `apps/victory/src/warm.rs` with `pipeline/known.rs`. Shader cache key: `shader_cache.rs` `build`.
- New replay experiment: a function in `replay/edit.rs`, a field in `play::Options`, applied in
  `Player::edited`, a flag in `apps/victory/src/replay.rs`.
- Something the recorder misses: a hook in `record.rs` `list_table` / `device_table` (out of
  `list_rest` / `device_rest`), a `Cmd` in `wire.rs`, a case in `Player::command`.
- Replay timing categories: `Player::translate`. Timeline content: `timeline::span` at the call
  site, kinds in `timeline/record.rs`, chains in `timeline/report.rs` `Trace::chain`.

## Origins

As source comments and commit messages give them:

- `ambient.rs`: where the stand-in goes and the crease shade are beloooo's (VIctory-Belobug-AMD
  06031237, 4f45ab88, cd9f78ce, GPL-3.0-or-later, as the module doc says).
- Screen-pass pipeline lane: after Belobug/VIctory-Belobug-AMD 18e57db8 (`make.rs`,
  `GraphicsState::screen_pass`).
- Root constant buffers copied once per list, and a rebound pipeline not kept again: commits c96525d
  and ab86ef7 (`draw.rs` with `host.rs`), from Belobug 84ea9c12 and 92d353f8 (romainhedouin).
- Dispatch counts as threads: DexNom/VIctory bc3cd4b0 (`draw.rs` `THREAD_DIMENSIONS`).
- Render targets not set again: after ZJOU acf478b8 (commit ca1d519).
- Draw-record limit in the clamp: victory-fork f85101b6 (`indirect.rs`).
- Staging budget from VRAM: fever308, 9beff3c (`draw.rs` `staging_budget_per_frame`).
- Bounded root SRVs: after upstream PR #493 (`pipeline/host.rs`); store detection after upstream #413
  (`pipeline.rs` `stores`); anisotropic samplers with point mips from upstream e45bacd, 23fc8a5,
  7dfa86f (`pipeline/host.rs`).
- Half subnormals in rectangle expansion: Enzo Souza Duarte, eb30a0d (`primitive.rs`).
- Much of the base came from fxceless/victory-fork commits and merged pull requests of
  Marvisak/VIctory; `git log` names each.
