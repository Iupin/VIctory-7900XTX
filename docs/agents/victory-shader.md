# victory-shader: map for agents

## Purpose

`crates/victory-shader` turns an Xbox Series shader container (DXBC with a
`DXIL` or `ACCC` part) into a PC DXIL container that host D3D12 accepts
(SM 6.6). It does not use the container's LLVM bitcode. It translates the
precompiled GFX10 machine code the console compiler stores beside the
bitcode. Callers: `victory-gpu` at pipeline creation (through its
`shader_cache`), and `apps/victory` (`translate`, `shaders`, `replay`).
The crate calls nothing of ours. It depends only on `thiserror`, `zlib-rs`,
`windows-sys`, and (feature `host`, or tests) `windows` for D3D12 checks.

Read first: the "Shaders" section of [`docs/architecture.md`](../architecture.md),
which sketches ir → structure → resource. Ray tracing as a feature is in
[`docs/fork-ray-tracing.md`](../fork-ray-tracing.md). Several module docs
cite "decision 0003". No decision document is in the repository.

## How it works

### The pipeline in one call

`emit::translate(container) -> Result<HostShader, ShaderError>` is the entry
point (`src/emit/mod.rs`). `translate_plain` runs these steps in order:

1. **Container**: `dxbc::Container::parse` reads the part table. The program
   part is `FourCC::DXIL`, or `FourCC::ACCC` when another compiler made the
   native code.
2. **Native programs**: `native::native_programs(part)` follows the `XDAT`
   index to the zlib-packed `XBCH` → `FSSM` blob. `unpack` and
   `inflate_zlib_exact` inflate it, refusing anything over `MAX_UNPACKED`.
   `programs` cuts out one `NativeProgram` per hardware stage slot (LS HS ES
   GS VS PS, mesh, compute). An `ACCC` part goes through `fspx_program`
   instead. A `NativeProgram` holds the stage, `wave64`, the VGPR count, the
   root layout (`Vec<RootParam>`: which user SGPRs hold which root
   parameter), the used-resource list (`Vec<UsedResource>`), the stage I/O
   (`StageIo`: PS inputs, VS inputs/outputs, `ComputeIo`, `MeshIo`) and the
   code. `translate_plain` takes the first VS, PS, CS or MS program. Merged
   stages (LS/HS/ES/GS) are not read. Unknown blob versions are errors, so a
   format change shows up.
3. **Export patch and spills**: `spill::build(native)` calls
   `ir::build(&native.patched_code(), ...)`. `NativeProgram::patched_code`
   writes each render target's export in its full 32-bit form, so exports
   carry unconverted values. Then `spill::user_data` handles a compute
   program whose root signature does not fit in its user SGPRs. Its
   parameters live in a memory table addressed by `s0`/`s1`, and the
   `s_load_dword*` loads from that table are rewritten into the entry values
   of the SGPRs their user-data index names. Only one layout is accepted:
   the table starts at user-data index 16 and the program has at least two
   registers. `spill::is_spilled` is true exactly for those programs. The
   module doc says that in the title these are the ray-tracing structure
   builder's programs. `victory-gpu` and `lower::counted_in_threads` key
   behaviour on it.
4. **Decoder** (`src/gfx10/`): `gfx10::decode` finds instructions by
   following control flow from offset 0, not by a linear sweep, because
   constant tables sit after the code. `control`/`far_jump` resolve
   `s_setpc_b64` far jumps. A branch that lands inside an instruction is an
   error. `Instruction` keeps its words and its name, which is looked up
   once in the Mesa-generated tables (`opcodes.rs`).
   `Instruction::fields(wave64)` decodes operands into `Fields`
   (`Valu`, `Sop`, `Smem`, `Buffer`, `Image`, `Flat`, `Exp`, ...).
   Register sizes come from the opcode name, and lane masks are sized by the
   wave size. `Instruction::text` prints AMD SP3 syntax, which is used by the
   app and by tests only.
5. **SSA IR** (`ir::build`): one `Inst` per machine instruction, except
   scheduling-only ones and conditional branches to the fall-through. Every
   register dword is a variable. SCC, M0 and EXEC are explicit reads. A VGPR
   write also reads the old value, because disabled lanes keep it.
   M0-relative VGPR access names the whole range up to `vgprs`. Before
   renaming, `unspill` turns SGPRs the compiler parked in VGPR lanes
   (`v_writelane`/`v_readlane` with constant lanes) into `s_mov`s to
   otherwise unused register codes (codes from `ir::PARKED` up cannot occur
   in machine words). This keeps a parked descriptor traceable and removes
   that cross-lane traffic. `split_irreducible` splits multi-entry loops.
   `rename` builds phis from liveness, and `remove_trivial_phis` cleans up.
   A branch to the entry block is refused. Program variants used by retries:
   `without_early_outs`, `without_skips(p, deep)` and `thread_edges`.
6. **Structurisation** (`structure::structure`, `structure_wide`): the
   program as one lane runs it. Lane masks become per-lane booleans, which
   are ROBDD nodes in `Bools` over `Atom`s (`Lane`, `Uniform`, `Phi`, `Quad`,
   `Opaque`, `Constant`). Wave-wide mask tests become per-lane branches
   (`LaneExit::Branch { when }`). A path-sensitive pass proves, for each
   vector instruction, that EXEC holds in every lane that reaches it. What
   it cannot prove stays as `Structured::guards[block][inst]`. The loop in
   `structure_wide` iterates `paths`/`check_joins` until the joins are
   consistent. BDD overflow gives `TOO_LARGE`, and an irreducible CFG is
   refused (`check_reducible`). `structure_wide(p, wide)` keeps the
   branches that close the blocks in `wide` as whole-wave decisions. That is
   for a console wave64 program run as two host waves of 32.
   `translate_plain` runs structurisation before resource tracing but
   reports its error after the resource and I/O errors.
7. **Resource tracing** (`resource::track(p, layout, stage, Some(&st))`):
   each descriptor operand (image, buffer, `s_buffer_*`, BVH) and each
   scalar or global memory address is traced through copies, agreeing phis
   and additions to a `Source` (`Loaded`, `Built`, `Constant`, `Phi`,
   `Select`, `Fetched`, `Lanes`, `LanePhi`) or an `Address { param, offset,
   dynamic }`. `param` is a layout index, `CODE` (a constant table after the
   code, read through `s_getpc`), or `MEMORY` (a raw title address; compute
   only, for the reasons in `RAW`). Waterfall loops (`v_readfirstlane` +
   `v_cmp_eq` over per-lane descriptors) are proven to serve every lane in
   one pass (`Waterfall`). `Resources::bvh` records for every
   `image_bvh*_intersect_ray` whether its nodes are a host buffer
   (`BvhNodes::Resource`) or title memory by address (`BvhNodes::Memory`,
   recognised by `bvh::DESCRIPTOR_TYPE` in dword 3). `root_indexed_views`
   lists heap descriptors whose index the draw can evaluate from root
   state.
8. **I/O raising** (`io::raise`): gives meaning to the entry VGPRs/SGPRs
   (`Input`), to `v_interp_*` pairs (`Interp`, `Barycentric`) and to exports
   (`Export`, `Target`). It covers PS, VS, CS and MS, and fails for other
   stages.
9. **Emission** (`emit::emit` → `emit_program` → `lower::lower`): reads the
   container's `ISG1`/`OSG1`/`PSG1` signatures (`signature`) and sets
   interpolation modes. For VS that includes `PSV0` via
   `vertex_output_interpolation`. Then it lowers and builds the `HostShader`.

### Lowering (`src/emit/lower.rs`)

`lower` → `lower_counted(parts, counted)` → `Lower::new(...).run(counted)`.
`run` creates one DXIL block per reachable IR block in reverse postorder.
It adds an iteration counter phi to every loop header (`LOOP_LIMIT` = 2^20,
overridable, see Switches), runs `find_sections` (the whole-wave sections of
`wave.rs`), then `inputs`, the mesh prologue, and `inst(b, i)` for every
instruction, then `exit(b)` and `fill_phis`. Every register dword is an
`i32` (SCC an `i1`). A value with no DXIL form becomes
`Val::Missing(reason)`, and the program fails only when something with an
effect needs it. `Stop::Retry(v, why)` re-runs the whole lowering with that
phi marked missing. If the same phi comes back, the error is "a phi that
does not settle". `counted` (from `counted_in_threads`: spilled 1-D compute
programs without `s_barrier`) puts an early return for lanes past a
host-supplied thread count (`USER_DISPATCH_THREADS_X`) ahead of everything.

`MaskData` decides how a lane mask read as a number is lowered:

- `Console` (compute, host lane = console lane, declares `WaveSize`): any use.
- `Host` (pixel and vertex): only wave-size-independent uses (bit count,
  first set bit, `v_mbcnt`, equality).
- `None` (mesh): only the uniform test.

Other uses stop with `LANE_MASK` or `MASK_ELSEWHERE`.

The helper submodules are these. `wave.rs` symbolically simulates
whole-wave sections (`s_or_saveexec -1` … restore) into DXIL wave
reductions and prefixes. `own_bits.rs` proves a wave OR only steers a
"walk my own bits" loop, so each host half-wave may use its own OR.
`intersect.rs` (a child module of `lower` via `#[path]`) inlines the BVH
node test. `threading.rs` (`constant_edges`) finds uniform diamonds for
jump threading. `sampler.rs` (`StaticSampler::from_words`) decodes immediate
sampler words into D3D12 static samplers.

### Wave64 on a 32-lane host: the retry ladder in `emit::emit`

`emit` runs `emit_program` once. If the result still needs 64-lane waves,
or fails with a reason `lower::needs_every_lane` accepts, and the program is
a wave64 CS on a host whose `host_max_wave() == 32`, it tries these forms in
order and returns the first that lowers without requiring wave 64:

1. `ir::without_early_outs`, then `ir::without_skips(p, false)`.
2. Exact local waves (`lower::with_local_waves`) where `lower::wave_local`
   proves the allocation idiom.
3. `without_skips(p, true)` ("deep").
4. `widen` (wave-wide branches through `structure_wide`, at two barriers a
   branch; only when `VICTORY_WAVE_WIDE=1` and the group is one console
   wave of 64).
5. Approximate local waves, then `with_local_waves_any` as the last resort.

Separately, a CS that fails with `LANE_MASK` is tried through `widen`
unless `VICTORY_WAVE_WIDE=0`. Finally, any remaining `Unsupported` result is
retried once with `threading::constant_edges` + `ir::thread_edges`.
`translate` adds one more outer retry. A refusal with `lower::FIRST_HALF`
is translated again under `lower::with_first_half_pairs`. `VICTORY_WAVE_WHY`
prints each step to stderr.

### Writing the output (`emit/shader.rs`, `emit/bitcode.rs`, `emit/container.rs`)

`Shader` (in `shader.rs`) wraps a `bitcode::Module` plus signatures,
bindings, flags, thread counts, mesh/geometry info and wave size.
`Shader::finish`:

1. fixes pixel input modes and the user-data buffer size;
2. runs the clean-up passes `Module::optimise(shader_opt() & opt::MODULE)`;
3. clears every feature flag whose last use the clean-up removed
   (`Module::usage`), because the validator compares flags with the code;
4. declares resources and metadata (`dx.shaderModel` 6.6, `dx.entryPoints`);
5. writes LLVM 3.7 bitcode with unabbreviated records only (`Module::write`;
   values are numbered at write time as functions, then constants, then
   `main`'s instructions);
6. wraps that in a DXIL program header (version 0x106, bitcode offset 16);
7. builds `SFI0` from the flags, `ISG1`/`OSG1`/`PSG1` (`container::signature`)
   and `PSV0` (`container::psv`);
8. calls `container::build`.

`container::build` lays the parts out and writes the D3D12 container
digest itself with `container::hash`, an MD5 variant with D3D's
last-block padding. No `dxil.dll` is needed at run time. `emit/validator.rs`
loads `dxil.dll` (`IDxcValidator`) for tests and the offline batch only.

### Clean-up passes and `VICTORY_SHADER_OPT`

Bits are in `src/emit/bitcode/opt.rs`. The value is parsed in
`emit::parse_shader_opt` (decimal or `0x` hex, masked with `opt::ALL`; unset
or unreadable means `opt::DEFAULT` = 63):

| Bit | Const | What |
|---|---|---|
| 1 | `DEAD` | dead-code elimination |
| 2 | `FOLD` | integer constant folding and identities (floats left alone) |
| 4 | `REUSE` | one instruction for one value (dominating equal instruction) |
| 8 | `FACTS` | inside a branch's side, its condition (and its terms) is constant |
| 16 | `LOADS` | one read per place within a block while nothing with an effect intervenes |
| 32 | `FIRST` | not a module pass: lowering of `v_readfirstlane_b32` in a wave64-as-two-waves program skips the "which half has a lane" exchange where every lane runs |
| 64 | `WIDE` | not a module pass: `raw_dwords` reads one console load four dwords per call; off by default (from fxceless/victory-fork c36971b2; measured as no gain) |

`MODULE` = 1|2|4|8|16 are the ones `Shader::finish` runs. `0` gives the
module exactly as lowered. The doc says every pass is exact, so the switch
exists to tell a translator fault from a clean-up fault and to measure.

### Spills, two kinds

- *Spilled user data* (`src/spill.rs`), described above.
- *Parked scalars* (`ir::unspill`): SGPRs saved in VGPR lanes. Register
  pairs in neighbouring lanes move as `s_mov_b64` when they are always saved
  and restored together.

### Ray tracing (BVH)

`src/bvh.rs` is the CPU reference for `image_bvh_intersect_ray` and
`image_bvh64_intersect_ray`. It covers node kinds by the low three pointer
bits, 16- and 32-bit box nodes with the sorting network (`boxes`), the
watertight triangle test (`triangle`, result `[t_num, denom, i_num, j_num]`,
`MISS` = +inf, 1, 0, 0), and the descriptor bits `BOX_SORT`,
`RETURN_BARYCENTRICS` and `DESCRIPTOR_TYPE`. `src/emit/intersect.rs`
(`Lower::bvh`) emits the same float operations in the same order. DXIL
carries no fast-math flags, so the result is comparable dword for dword
(`intersect_tests.rs`). Nodes in title memory are read through the host's
copies of title memory (`Lower::memory`, the `SLOT_MEMORY` directory). The
structure builder's own programs are the spilled compute programs. Their
root views are reached as memory by address (`USER_ROOT_ADDRESS`,
`USER_VIEW_BIAS`). Their buffer atomics are fenced and their accesses
bounded unless `VICTORY_BVH_FENCES=0` / `VICTORY_BVH_BOUNDS=0`.

### What the host must bind (`HostShader`)

The `emit/mod.rs` module doc is the contract. In short:

- user data is a `b0` space-0 buffer (`HostShader::user_data`: `UserData {
  index, slot, root, dword }`; root constants must cover `user_data_dwords()`);
- descriptors come from `ResourceDescriptorHeap[]`/`SamplerDescriptorHeap[]`
  at the console's index;
- immediate samplers are static samplers `s<n>`;
- root CBV/SRV/UAVs sit in `ROOT_CBV_SPACE` by root parameter index;
- reserved root slots are `SLOT_STRIDES` (255, per-heap-slot buffer
  strides), `SLOT_MEMORY` (254, the title-memory page directory) and
  `SLOT_FAULTS` (253, pages that had no copy);
- host-computed user-data kinds are `USER_MEMORY_BASE`, `USER_VIEW_ADDRESS`,
  `USER_VIEW_BIAS`, `USER_ROOT_ADDRESS` and `USER_DISPATCH_THREADS_X`, all
  in `shader.rs`.

`victory-gpu` (`draw.rs`, `memory.rs`, `descriptor/host.rs`,
`pipeline/host.rs`) imports these constants. Change both sides together.

### Process-wide host facts

`victory-gpu` (`host.rs`, `descriptor/host.rs`) and `apps/victory` set these
before translating:

- `emit::set_host_max_wave` (`WaveLaneCountMax`; 32 enables the wave64
  retries);
- `emit::set_host_rdna` (enables the `VICTORY_RDNA_FLOAT` forms);
- `emit::set_view_heap_limit` (clamp heap indices to a null slot);
- `emit::set_raw_views` (where raw twins of heap buffer views start).

They are atomics read at translation time. `emit::host_settings()` returns
them for cache keys. Per-attempt state (`NEEDS`, `LOCAL_WAVES`,
`FIRST_HALF_PAIRS`) is thread-local, so translations on several threads at
once are independent.

### Errors: how an unhandled shader is reported

`ShaderError` (in `src/lib.rs`) has four variants:

- `NotDxbc`;
- `Malformed(&'static str)`;
- `Isa { offset, reason }` (undecodable code);
- `Unsupported { offset, what }` (valid code the translator cannot handle
  yet).

`what` is a short fixed phrase, so failures can be tallied by reason.
Lowering reasons are the `Why` strings, for example `lower::LANE_MASK`.
The retries above trigger only on `Unsupported`. In `victory-gpu`,
`pipeline.rs` `Shader::translated` logs a translation failure once ("shader
does not translate; its pipelines' draws are dropped") and the draws are
dropped. `apps/victory/src/translate.rs` counts failures by stage and
reason (`Tally::fail`, `translate_programs`).

### Caching

The crate keeps no cache of its own. Two fingerprint mechanisms exist:

- `build.rs` hashes this crate's `src/**/*.rs` (FNV-1a) into
  `VICTORY_SHADER_SOURCE_HASH`, collects every `"VICTORY_*"` literal into
  `VICTORY_SHADER_SWITCHES`, and `emit::translation_fingerprint()` combines
  those with the switch values, host settings, loop limit, group sync and
  opt bits. **No caller uses `translation_fingerprint` today.**
- What is used: `crates/victory-gpu/src/shader_cache.rs` `build()` keys on
  `VICTORY_TRANSLATOR` (BLAKE3 of this crate's `src`, its `Cargo.toml`,
  `Cargo.lock`, the toolchain file and `shader_cache.rs`, from
  `crates/victory-gpu/build.rs`), plus `loop_limit()`, `group_sync()`,
  `shader_opt()`, `host_settings()` and a **hand-written list** of switch
  names. Entries go through `victory-pipeline`'s content-addressed cache.

### The bench

`benches/guard_runs.rs` (`harness = false`) times `structure::structure`
only. It builds three synthetic programs with `ir::build`:

- "unmasked" (512 `v_add`);
- "short runs" (8 `v_cmpx` each followed by 1 add);
- "long runs" (8 `v_cmpx` each followed by 64 adds).

It prints the median of 9 samples of 64 runs each. It needs no game files
and no GPU. Run it with `cargo bench -p victory-shader`.

### Tests and adding a case without game data

Unit tests sit in most modules; `resource.rs`, `emit/mod.rs` and
`emit/valu_tests.rs` hold the most. They build programs from hand-encoded
GFX10 words (encoder helpers such as `vop1`, `s_mov_b32` and
`buffer_store_dword` in `valu_tests.rs`, and `resource::tests::cmpx_waterfall`)
or synthetic containers (`dxbc::build`, `dxil::build_standard`). No test
reads game files. The GPU tests are Windows-only modules (`*_tests.rs`
under `emit/`, `cfg(all(test, windows))`). They lower, validate with
`dxil.dll` when it is found, run on the hardware adapter and on WARP, and
skip with a message when no SM 6.6 device exists. They serialise on
`valu_tests::ONE_AT_A_TIME`.

To add a VALU or compute case, copy the pattern of
`valu_tests.rs::build_cs`. Write the instruction words. Wrap them in a
`NativeProgram` with a literal layout (root constants in `s0..s3`, a UAV
table in `s4`). Run `ir::build` → `structure` → `resource::track` →
`io::raise` → `lower::lower`, then `run_cases` with input dwords and check
the stored dword against a reference computed in Rust. For node tests,
compare against `crate::bvh`.

`diff_tests.rs` compiles HLSL held in the file (the `CASES`, `CS_CASES` and
`MS_CASES` arrays) with the console SDK's `dxc.exe` (`VICTORY_CONSOLE_DXC`)
and a PC `dxc.exe`, then draws both. A new HLSL case is a new entry there.
The compiled outputs stay in a temp folder. Asset policy
([`docs/asset-policy.md`](../asset-policy.md)): never commit a shader
container, dump or disassembly of the title. Tests assert on properties,
never on snapshots of real shaders.

## Files

| Path | What it does | Key items |
|---|---|---|
| `crates/victory-shader/Cargo.toml` | Manifest. Features: `host` (D3D12 checks in `emit::host`), `diagnostics` (enables `VICTORY_SHADER_LOOP_LIMIT`). Declares the bench. | `[features]`, `[[bench]] guard_runs` |
| `crates/victory-shader/build.rs` | Hashes `src/**/*.rs` and collects every `"VICTORY_*"` literal for `translation_fingerprint`. | `VICTORY_SHADER_SOURCE_HASH`, `VICTORY_SHADER_SWITCHES` |
| `crates/victory-shader/benches/guard_runs.rs` | Times structurisation of three synthetic programs. | `program`, `main` |
| `crates/victory-shader/src/lib.rs` | Module list, re-exports, the error type. | `ShaderError` |
| `crates/victory-shader/src/dxbc.rs` | DXBC container parser, plus a test builder. | `Container::parse`, `Container::part`, `FourCC` (`DXIL`, `ACCC`, `PSV0`, ...), `u32_at` |
| `crates/victory-shader/src/dxil.rs` | The DXIL program header around the bitcode; detects the Xbox `XDAT`/`xbop` additions. Used by the app's survey, not by `translate`. | `DxilProgram::parse`, `ShaderKind`, `XboxExtras` |
| `crates/victory-shader/src/native.rs` | Reads the console's native programs (`XDAT` → `XBCH` zlib → `FSSM` records; `ACCC`/`FSPX`): code, root layout, used resources, stage I/O, export patches. | `native_programs`, `fspx_program`, `NativeProgram`, `patched_code`, `HwStage::user_sgpr`, `RootParam`, `RootType`, `UsedResource`, `StageIo`, `ComputeIo`, `MeshIo` |
| `crates/victory-shader/src/spill.rs` | Rewrites a compute program's loads from its spilled user-data table into entry SGPR values. | `build`, `user_data`, `is_spilled` |
| `crates/victory-shader/src/gfx10/mod.rs` | GFX10 decoder: encodings, control-flow-driven decode, far jumps. | `decode`, `Instruction`, `Encoding`, `Flow`, `control` |
| `crates/victory-shader/src/gfx10/fields.rs` | Operand fields per encoding, with register sizes taken from opcode names and wave size. | `Instruction::fields`, `Fields`, `Operand`, `Valu`, `Sdwa`, `Dpp`, `Cache`, `NULL` |
| `crates/victory-shader/src/gfx10/opcodes.rs` | Opcode name tables, sorted, generated from Mesa's `aco_opcodes.py` (mesa-22.3.0) with its MIT notice. | `SOP2`, `VOP3`, ... tables |
| `crates/victory-shader/src/gfx10/sp3.rs` | Prints instructions in AMD SP3 syntax. | `Instruction::text` |
| `crates/victory-shader/src/ir.rs` | One-to-one SSA IR; parked-scalar unspill; irreducible loop split; program variants for retries. | `build`, `Program`, `Block`, `Inst`, `Phi`, `Reg`, `Value`, `Exit`, `unspill`, `without_early_outs`, `without_skips`, `thread_edges`, `reverse_postorder` |
| `crates/victory-shader/src/structure.rs` | EXEC removal: per-lane booleans (ROBDDs), per-lane branches, guards. | `structure`, `structure_wide`, `Structured`, `Bools`, `Bool`, `Atom`, `LaneExit` |
| `crates/victory-shader/src/resource.rs` | Traces descriptors and memory addresses to root parameters; waterfall proofs; BVH node sources. | `track`, `Resources`, `Source`, `Address`, `CODE`, `MEMORY`, `Descriptor`, `Role`, `Waterfall`, `BvhNodes`, `RootIndexedView`, `Site` |
| `crates/victory-shader/src/io.rs` | Raises entry registers, interpolation and exports to meanings. | `raise`, `Io`, `Input`, `Interp`, `Barycentric`, `Export`, `Target` |
| `crates/victory-shader/src/bvh.rs` | CPU reference of the ray/node test and the node formats. | `intersect`, `boxes`, `triangle`, `node_address`, `kind`, `DESCRIPTOR_TYPE`, `BOX_SORT`, `RETURN_BARYCENTRICS`, `MISS`, `NONE` |
| `crates/victory-shader/src/emit/mod.rs` | Public API and binding contract; switches; the wave64 retry ladder; signatures; many tests. | `translate`, `emit`, `HostShader`, `UserData`, `shader_opt`, `loop_limit`, `group_sync`, `helper_own`, `rdna_float`, `rdna`, `builder_fences`, `builder_bounds`, `translation_fingerprint`, `widen`, `emit_program`, `signature` |
| `crates/victory-shader/src/emit/lower.rs` | IR → DXIL instruction by instruction (about 13,000 lines): values, guards, memory, descriptors, wave ops, interpolation, exports, loops; host facts. | `lower`, `lower_counted`, `Lower::run`, `Lower::inst`, `Parts`, `Lowered`, `MaskData`, `LOOP_LIMIT`, `LANE_MASK`, `MASK_ELSEWHERE`, `FIRST_HALF`, `with_local_waves`, `wave_local`, `set_host_max_wave`, `set_host_rdna`, `host_settings` |
| `crates/victory-shader/src/emit/intersect.rs` | `image_bvh*_intersect_ray` as inline DXIL (child module of `lower`). | `Lower::bvh`, `TEST_MEMORY` (tests) |
| `crates/victory-shader/src/emit/wave.rs` | Symbolic simulation of whole-wave sections into reductions and prefixes. | `sections`, `Section`, `Shape`, `Sym`, `Op` |
| `crates/victory-shader/src/emit/own_bits.rs` | Proves a wave OR only drives a loop over each lane's own bits. | `find`, `OwnBits` |
| `crates/victory-shader/src/emit/threading.rs` | Finds uniform diamonds whose arm fixes the next branch, for jump threading. | `constant_edges` |
| `crates/victory-shader/src/emit/shader.rs` | DXIL module under construction: handles, flags, metadata, the finished container; reserved slots and user-data kinds; host heap facts. | `Shader`, `Shader::finish`, `flags`, `SLOT_STRIDES`, `SLOT_MEMORY`, `SLOT_FAULTS`, `USER_*`, `ROOT_CBV_SPACE`, `set_view_heap_limit`, `set_raw_views` |
| `crates/victory-shader/src/emit/bitcode.rs` | DXIL module model and LLVM 3.7 bitcode writer (unabbreviated records). | `Module`, `Module::write`, `V`, `Ty`, `Bin`, `Cast`, `Term`, `Tag` |
| `crates/victory-shader/src/emit/bitcode/opt.rs` | Exact clean-up passes and the `VICTORY_SHADER_OPT` bits; feature usage for flags. | `Module::optimise`, `Module::usage`, `DEAD`, `FOLD`, `REUSE`, `FACTS`, `LOADS`, `FIRST`, `WIDE`, `MODULE`, `ALL`, `DEFAULT` |
| `crates/victory-shader/src/emit/bitcode/census.rs` | Diagnostic counts of a written module's instructions by kind. | `enabled`, `take`, `note`, `Census`, `KINDS` |
| `crates/victory-shader/src/emit/container.rs` | Builds the PC container (`SFI0`, signatures, `PSV0`, `DXIL`) and its D3D12 hash. | `build`, `hash`, `signature`, `psv`, `SigElement`, `MESH_OUTPUT_LIMIT` |
| `crates/victory-shader/src/emit/sampler.rs` | Console sampler words → D3D12 static sampler. | `StaticSampler`, `from_words`, `filter_for_host` |
| `crates/victory-shader/src/emit/rect.rs` | Our geometry shader that completes rectangle lists. | `rect_list_gs` |
| `crates/victory-shader/src/emit/host.rs` | D3D12 test host (feature `host` or tests, Windows): pipelines, draw, dispatch, read-back. | `Host::new`, `compute_pipeline`, `pipeline`, `mesh_pipeline`, `dispatch`, `draw`, `stub_vs`, `stub_ps` |
| `crates/victory-shader/src/emit/validator.rs` | Loads `dxil.dll` and validates containers (tests and offline batch). | `Validator::default_path`, `load`, `validate` |
| `crates/victory-shader/src/emit/valu_tests.rs` | GPU tests of synthetic VALU/compute programs against Rust references (33 tests). | `build_cs`, `run_cases`, `ONE_AT_A_TIME` |
| `crates/victory-shader/src/emit/diff_tests.rs` | Differential tests: HLSL by the console dxc vs PC dxc, both run on the host. | `CASES`, `CS_CASES`, `MS_CASES`, `tools`, `compile` |
| `crates/victory-shader/src/emit/intersect_tests.rs` | BVH node test on the GPU vs `crate::bvh`. | 6 tests |
| `crates/victory-shader/src/emit/waterfall_tests.rs` | Waterfall loops over per-lane descriptors, end to end. | 6 tests |
| `crates/victory-shader/src/emit/wave64_tests.rs` | A wave64 program on 32-lane waves with wave-wide branches. | 9 tests |
| `crates/victory-shader/src/emit/size_tests.rs` | Descriptors read for sizes and counters, end to end. | 8 tests |
| `crates/victory-shader/src/emit/permlane_tests.rs` | Synthetic row permutations on hosts of at least 32 lanes. | 2 tests |
| `crates/victory-shader/src/emit/cube_lod_tests.rs` | Cube sampling with gradients picks the level by footprint. | 1 test |
| `crates/victory-shader/src/emit/probe_tests.rs` | Not a translator test: runs `.dxil` files from `VICTORY_PROBE` on the card (`--ignored probe`). | 1 test |

## Switches

All are read once (`OnceLock`) unless noted. Those marked "key" change
the output and are part of `victory-gpu`'s cache key.

| Name | Default | What it changes | Read in |
|---|---|---|---|
| `VICTORY_SHADER_OPT` | 63 | clean-up/lowering bits (table above). Key. | `src/emit/mod.rs` `shader_opt` |
| `VICTORY_SHADER_LOOP_LIMIT` | 2^20 | iterations before a translated loop is left; only with feature `diagnostics`. Key. | `src/emit/mod.rs` `loop_limit` |
| `VICTORY_GROUP_SYNC` | 2 | `2`: every barrier waits; `1`: `s_barrier` waits only where every lane runs; `0`: no barrier waits. Key. | `src/emit/mod.rs` `group_sync` |
| `VICTORY_HELPER_OWN` | on | `0`: helper lanes read the first lane's value as the console does. Key. | `src/emit/mod.rs` `helper_own` |
| `VICTORY_BVH_FENCES` | on | `0`: structure builder's buffer atomics not fenced. Key. | `src/emit/mod.rs` `builder_fences` |
| `VICTORY_BVH_BOUNDS` | on | `0`: builder's root-view accesses not bounded. Key. | `src/emit/mod.rs` `builder_bounds` |
| `VICTORY_RDNA_FLOAT` | 15 (all) | on RDNA hosts, which console float ops use the single host op (`rdna::MAX_MIN`, `LEGACY`, `MED3`, `FRACT`). Key via `host_settings`. | `src/emit/mod.rs` `rdna_float` |
| `VICTORY_WAVE_LOCAL` | on | `0`: no local-wave forms; `idiom`: only the exact ones. Key. | `src/emit/mod.rs` `emit` (read per call) |
| `VICTORY_WAVE_WIDE` | off for wave64 retry; on for the `LANE_MASK` retry | `1`: try wave-wide branches for wave64; `0`: also off for the 32-lane `LANE_MASK` retry. Key. | `src/emit/mod.rs` `emit` (per call) |
| `VICTORY_WAVE_DEEP_SKIPS` | on | `0`: no `without_skips(p, true)` form. Key. | `src/emit/mod.rs` `emit` (per call) |
| `VICTORY_WAVE_WHY` | off | prints retry and wave-size reasons to stderr. | `src/emit/mod.rs`, `src/emit/lower.rs` |
| `VICTORY_WAVE_OWN_BITS` | on | `0`: no `own_bits` proof. Key. | `src/emit/lower.rs` `own_bits_on` |
| `VICTORY_F16_NEAREST` | off | `1`: 16-bit float results rounded to nearest as the console does. Key. | `src/emit/lower.rs` `f16_nearest_on` |
| `VICTORY_FIRST_HALF` | on | `0`: the first-half-pair retry is refused again. Key. | `src/emit/lower.rs` (per call) |
| `VICTORY_FIRSTLANE_OWN` | off | `1`: a host wave's own first lane where the halves cannot exchange (approximate). Key. | `src/emit/lower.rs` |
| `VICTORY_STRICT_JOINS` | off | set: a join reading a value with no DXIL form fails instead of becoming zero. Key. | `src/emit/lower.rs` `fill_phis` |
| `VICTORY_MESH_OUTPUT_STRICT` | off | set: refuse mesh shaders whose outputs pass 32 KiB even at the lane count. Key. | `src/emit/lower.rs` (per call) |
| `VICTORY_DESC_WHY` | off | prints descriptor tracing details to stderr. | `src/emit/lower.rs` (per call) |
| `VICTORY_SHADER_CENSUS` | off | counts instructions by kind for `census::take`. | `src/emit/bitcode/census.rs` `enabled` |
| `VICTORY_DXIL` | SDK path | path of `dxil.dll` for the validator. | `src/emit/validator.rs` |
| `VICTORY_WARP`, `VICTORY_D3D12_DEBUG` | off | test host: WARP adapter; D3D12 debug layer. | `src/emit/host.rs` `Host::new` |
| `VICTORY_CONSOLE_DXC`, `VICTORY_PC_DXC`, `VICTORY_DIFF_ONLY`, `VICTORY_EMIT_DUMP` | unset | diff tests: compilers, case filter, dump folder. | `src/emit/diff_tests.rs` |
| `VICTORY_REF_PS`, `VICTORY_REF_VS` | unset | test `stub_pipeline_is_created`: use these DXIL files. | `src/emit/mod.rs` tests |
| `VICTORY_PROBE` | unset | folder of `.dxil` files for `probe_tests`. | `src/emit/probe_tests.rs` |

`VICTORY_SHADER_SOURCE_HASH` and `VICTORY_SHADER_SWITCHES` are compile-time
variables set by `build.rs`, not switches.

## Traps

- **Cache-key drift.** A new switch that changes output must be added to the
  hand list in `crates/victory-gpu/src/shader_cache.rs` `build()`, or a run
  with it set is served old translations. `build.rs` collects names
  automatically, but only for the unused `translation_fingerprint`.
  `VICTORY_SHADER_LOOP_LIMIT` is handled separately there (through
  `loop_limit()`).
- **Host facts before translation.** `set_host_max_wave`, `set_host_rdna`,
  `set_view_heap_limit` and `set_raw_views` change the output. They must be
  set before the first translation, and they enter the key through
  `host_settings()`. `victory-gpu` calls `probe_view_limit` before keying.
- **Most switches are latched** in `OnceLock`s. Several `emit`/`lower`
  switches are read on every call (marked "per call" above).
- **Binding contract.** Changes to `HostShader`, reserved slots
  (`SLOT_*`), `USER_*` kinds or spaces must be mirrored in `victory-gpu`.
  The cache entry is `victory-gpu`'s own encoding of `HostShader` (its
  module doc says new fields must be added to `encode`/`decode`).
- **Flags must match usage.** `Shader::finish` clears feature flags after
  the clean-up. A new feature flag needs a `Usage` field in `opt.rs`, or the
  validator (and D3D) reject the shader whenever a pass removes the last
  use.
- **Container hash.** D3D12 refuses a wrong or zero digest (tested in
  `stub_pipeline_is_created`). Any byte change after `container::build`
  needs `container::hash` again.
- **Exactness.** The `opt` passes claim exactness. Float arithmetic is
  deliberately not folded. `intersect.rs` relies on DXIL having no fast-math
  flags to match `bvh.rs` bit for bit. Keep the two in step.
- **Decode by control flow.** Data after the program is never decoded. An
  "unknown opcode" in data would be a decoder bug, not a translator bug.
- **Error order in `translate_plain`.** Structurisation runs first, but its
  error is reported after resource tracing and I/O raising. Resource
  tracing without the per-lane form skips waterfall proofs.
- **Retries change behaviour.** Some retry forms are approximate (log label
  "APPROXIMATE"). `VICTORY_WAVE_WIDE=1` is off by default because a comment
  in `emit` records device losses with it.
- **`Missing` values.** A value with no DXIL form only fails when an
  effect needs it. Without `VICTORY_STRICT_JOINS` such a value reaching a
  join silently becomes zero (a known `shortcut` in `fill_phis`).
- **`shortcut:` comments** mark knowingly approximate choices (loop limit,
  mesh output caps, rectangle flat vertex, local waves). Search for
  `shortcut` before assuming exactness.
- **Asset policy in sources.** Some existing comments name title programs.
  Do not add shader names, dumps or code words taken from the title. Test
  words must be hand-encoded.
- **GPU tests** serialise on `ONE_AT_A_TIME` (concurrent WARP use crashed
  or hung). A new GPU test module should take it.

## Where to start for common changes

- A new GFX10 instruction: name in `gfx10/opcodes.rs` (if missing),
  operand sizes in `gfx10/fields.rs` (`salu_dwords`, `valu_dwords`, ...),
  register effects in `ir.rs` `effects`, lowering in `emit/lower.rs`
  (`Lower::inst` → `valu_op`, `sop`, `image`, `buffer`, `ds`, `smem`, ...),
  a test in `emit/valu_tests.rs`.
- "a lane mask read as data" / wave64 refusals: `emit/lower.rs`
  (`MaskData`, `reads_mask_as_data`, `wave_local`), `emit/wave.rs`,
  `emit/own_bits.rs`, and the retry ladder in `emit/mod.rs` `emit`.
- A descriptor that does not trace: `resource.rs` `Tracer` (`descriptor`,
  `smem_address`, `by_address`). `VICTORY_DESC_WHY` helps.
- A new input/output semantic: `io.rs` `raise`, `emit/mod.rs` `signature`,
  `emit/container.rs` `SigElement::system_value`, `psv`.
- A new console container layout: `native.rs` (`programs`, `semantics`,
  `fspx_program`).
- A new clean-up pass: `emit/bitcode/opt.rs` (bit, `MODULE`/`DEFAULT`,
  `Module::optimise`), the doc of `emit::shader_opt`, tests in `opt.rs`.
- A new output-changing switch: read it in this crate, then add it to
  `victory-gpu/src/shader_cache.rs` `build()`.
- Ray tracing node tests: `bvh.rs` (reference) and `emit/intersect.rs`
  (code) together, with `emit/intersect_tests.rs`.
- Why the offline batch refuses something: run `victory translate`
  (`apps/victory/src/translate.rs`). It tallies `ShaderError::Unsupported`
  by `what`.

## Origins

- The crate began in Marvisak/VIctory (`f4504f6` "Initial commit",
  `12caefe`, `754619f`, author Marvis). Many contributors' pull requests
  were merged there, as `git log -- crates/victory-shader` shows.
- fxceless/victory-fork is the base of this fork (`578d459`, author
  fxceless, "this fork's changes on top of VIctory main (ffe310e)"). Later
  fxceless commits include the clean-up passes (`1684989`).
- `gfx10/opcodes.rs`: generated from Mesa's `src/amd/compiler/aco_opcodes.py`
  (mesa-22.3.0), Copyright (c) 2018 Valve Corporation, MIT notice kept in
  the file.
- `bvh.rs`: node formats and tests follow the public software emulation in
  the open-source Vulkan driver for this GPU family. The triangle test is
  the watertight one of Woop, Benthin and Wald (JCGT 2013).
- Credits in source comments:
  - DexNom/VIctory `5f4c3df4` (vertex output interpolation, `emit/mod.rs`),
    `bc3cd4b0` (threads counted per dispatch; missing children in
    `bvh.rs`), `921d8d3b` (cube LOD, base-2 log);
  - fxceless/victory-fork `c36971b2` (`opt::WIDE`);
  - VIctory `4243a96a` (integer views, `VIEW_INTEGER`), `a1a2077a`
    (`UAVS_EVERY_STAGE`), `e29391f` (f16 interpolation halves), upstream
    `0b35b7f` (`ACCC`, `src_scc`, raw views).
