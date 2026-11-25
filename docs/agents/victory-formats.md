# victory-formats

## Purpose

Platform-level data formats shared by every Xbox Series title, as pure code
(bytes in, bytes out, no I/O, no dependencies): the console GPU's texture
layouts (`texture`), the BCPack texture packing that DirectStorage unpacks in
hardware (`bcpack`), and the XMA2 audio codec (`xma`). Callers:
`victory-gpu` (`resource.rs`, `texture.rs`, `texback.rs` use
`texture::Layout`), `victory-hle::modules::xg` (texture sizes the title asks
for), `victory-hle::modules::dstorage` (`bcpack::decode`) and
`victory-hle::modules::audio` (`xma`). Opus is not here: the console's
`xapu.dll` decoder is in `crates/victory-hle/src/modules/apu.rs` on the
`opus-pure` crate.

## How it works

### Texture layouts (`crates/victory-formats/src/texture.rs`)

Follows AMD's addrlib (Mesa `src/amd/addrlib`, MIT, notice kept in the module
docs) for GFX10.3. Units: an *element* is a texel, or a 4x4 block for BC
formats. Inside one array slice (or one depth slab of a thick 3D texture) the
mips are stored smallest first: the mip tail at offset 0, mip 0 last.

Two levels of description:

- `TextureDesc` is what the console API gives: dimension, extent, `mips`
  (0 = full chain), DXGI format number, samples, `Swizzle`. `Texture::new`
  maps the format through `format(dxgi)` to a `Format { bytes, block, stencil }`
  and builds one `Plane` per data plane: depth (or colour) first and, for
  depth-stencil formats, a 1-byte stencil plane placed after it at its own
  alignment. `Texture { planes, size, alignment }`.
- `Desc` is one plane: element bytes (1-16, power of two), `block` (1 or 4),
  `depth` (3D depth or array size). `Layout::new(&Desc)` validates and builds
  a `Layout`.

`Layout::new` rejects (`Error::Invalid`) zero or over-`MAX_EXTENT` (16384)
extents, more than `MAX_MIPS` (16), bad element sizes, MSAA unless 2D,
single-mip and tiled, 1D with height above 1, and 3D with a mode 3D cannot use.
Then:

- `Layout::linear` (`HwlComputeSurfaceInfoLinear`): rows aligned to 256 bytes,
  mips packed smallest first, each slice holds the chain, alignment 256.
- `Layout::tiled`: block dimensions from `ComputeThickBlockDimension` (3D `S`/`D`
  modes are *thick*: a block spans `block[2]` depth slices) or
  `ComputeThinBlockDimension`. With one mip the slice is one padded mip. With
  more, it computes the largest mip the tail can take (`GetMipTailDim`), how
  many mips fit (`GetMaxNumMipsInTail`; 256-byte blocks have no tail), applies
  the 8/16-bit depth fix for `Z` order, walks mips from 0 until one fits the
  tail, then places non-tail mips from the smallest up after the tail block,
  and gives each tail mip a `Tail { offset, x, y }` inside the tail block.
  `Mip` carries real size (`width/height/depth`) and padded size
  (`pitch/rows/depth_pitch`); `dim` vs `padded_dim` differ for odd sizes
  (addrlib rounds mip 0 up before halving).

Addressing (`Layout::offset(mip, x, y, z)`): base = slice * `slice_size` +
mip offset, then `row` (block row times blocks per row, plus the in-block bits
for y and z) plus `column` (block column plus in-block bits for x). The
in-block bits come from `pattern`, a per-axis table of byte offsets built from
the 256-byte micro-tile strings in `MICRO` (addrlib's RB+ patterns) and then
extended with alternating Y, X (thin) or Y, X, Z (thick) bits up to the block
size (`pattern`, `spread`). Tables are shared and lazily built per
(kind, block size, element size) in `shared_pattern` (`OnceLock` statics), so
a layout holds a `&'static` reference.

`Layout::addressable()` is false for the pipe/bank XOR modes (`_X`, `_T`,
`Swizzle::xor`) and MSAA: their patterns are empty and `offset`, `tile` and
`detile` return `Error::Unsupported`. That is a deliberate shortcut: titles use
those modes for render targets and depth, which the host renders.

Detiling (`Layout::detile` / `Layout::tile`, both `walk::<DETILE>`): one
subresource (mip of one array slice, or all depth slices of a 3D mip with
`slice` 0) to or from linear rows at `row_pitch` / `depth_pitch`. The hot path:
because row and depth only add a common base, the contiguous runs of a row are
computed once from `column` for x in `0..width`; if all runs but the last
have the same length 8, 16 or 32 and aligned offsets, rows are copied with
`copy_runs::<N>` over `as_chunks` arrays (one bounds check per chunk), else
byte ranges. Size checks happen up front: the tiled side must be at least
`bytes_from(mip)` for detile (the whole `size` for tile), the linear side must
hold `(depth-1)*depth_pitch + (height-1)*row_pitch + width*bytes`, and
`depth_pitch` must not let depth slices overlap.

`Layout::bytes_from(first)` is how many bytes from the texture start mip
`first` and smaller need. It lets a streamed texture whose memory holds only
the small mips be detiled from its start. It returns the whole size for
arrays, thick or 3D layouts, MSAA and `first == 0`.

`texture::format` covers DXGI formats by number; 96-bit, 1-bit, packed 4:2:2
and video formats return `None`. `Texture` has no compression metadata planes
(HTILE, CMASK, FMASK, DCC); console sizes of render targets are therefore
smaller here (documented shortcut).

### BCPack (`crates/victory-formats/src/bcpack.rs`)

Decode only, BC1 and BC3-BC7 (BC2 unsupported). A stream has no header and no
length. `decode(bc, src, dst, tables)` takes the block count from `dst.len()`.

1. **Lanes.** The stream is 12 (BC1/BC4) or 16 (others) interleaved rANS lanes.
   `lanes` steps every `Lane` in order each round; a fixed schedule lets the
   lanes between them take at most `ROUND_BYTES` (8) input bytes per round. A
   lane reads a fragment header byte (which picks one of 128 static models and
   flags reset/long length), a length (one or two bytes), then payload bytes
   that feed the rANS state; `Lane::step` decodes at most one symbol per step
   using the model's bucket frequencies (`bucket_of` with a 256-slot window
   table, then an exact reciprocal division). Symbols land in a per-block
   record of `widths.len() * per` bytes. If nothing is read or delivered for
   more than three rounds, the stream is reported as ending early or corrupt.
2. **Blocks.** Each record holds two back references (to one of the last 256
   blocks, `HISTORY`, or to a per-format dictionary that stands in for blocks
   -1 to -255) and residuals of endpoints and indices against them.
   `plain` handles BC1/3/4/5 (`*_residual`, `*_pack`), `bc6` and `bc7` the
   two complex formats, using the public BC6H/BC7 bit layouts kept as our own
   tables (`BC6_LAYOUT`, `BC7_BITS`, anchors) and the format's rank tables and
   BC7 control words.

The hot path is `lanes`/`Lane::step`, single-threaded; the comment gives
roughly 30-60 MB/s for BC1 and 15-20 MB/s for BC7 on one core and names the
next step if streaming needs more (split input per lane first, decode lanes
apart).

**The tables file.** `crates/victory-formats/src/bcpack-tables.bin` holds the
format's constant tables: 128 x 9 model frequencies, the rank tables, 512 BC7
control words and 255 dictionary entries per format; layout in the doc of
`Tables::parse`, size `TABLES_SIZE`. `Tables::builtin` embeds it with
`include_bytes!`. It is the repository's one asset-policy exception (see
[docs/asset-policy.md](../asset-policy.md), "The one exception"): it is not
our work, the licence does not cover it, and it is entered in
`xtask/src/policy.rs` `EXCEPTIONS` and un-ignored in `.gitignore`. `parse`
checks the exact size and that every model tiles 65536.
`victory-hle::modules::dstorage` uses `Tables::builtin()` at runtime; under
`cfg(test)` it parses a `tables.bin` from a folder the test sets
(`use_bcpack_tables`) instead.

### XMA2 (`crates/victory-formats/src/xma.rs`, `crates/victory-formats/src/xma/tables.rs`)

XMA2 is mono or stereo WMA Pro frames of 512 samples per channel in 2048-byte
packets (`PACKET_BYTES`, `FRAME_SAMPLES`).

- `packet(data, index)` reads a packet's 32-bit big-endian header into
  `Packet { frames, first_frame_bit, skip }` (`skip` = packets of other
  streams before this stream's next one).
- `read_frame(data, start_bit)` gathers one frame's bits (its 15-bit length
  first) across packet boundaries with `Cursor`, which jumps over each packet
  header and the other streams' packets, and returns `Frame { data, bits,
  start, next }`. `next_frame` finds where the next frame starts: right after,
  or (last frame of a packet, padding, or a frame ending in a packet with no
  frame starts) at the next packet of this stream that has frames. A length
  that does not cover its own field is not a frame; this ends what used to be
  endless loops on damaged data.
- `Decoder::new(channels, sample_rate)` precomputes scale-factor band layouts
  for the three subframe sizes (512, 256, 128). `Decoder::decode(data, bits,
  out)` decodes one frame and appends 512 interleaved `i16` samples per
  channel; on an error it still appends silence so stream timing holds, and
  returns the error. `flush` emits the remaining overlap; `reset` forgets it.
  Internals follow FFmpeg's `wmaprodec.c`: `tile_header`, `subframe`,
  `channel_transform`, `scale_factors`, `coefficients` (VLC tables from
  `xma/tables.rs`, built once in `codes()`), `imdct_half` (FFT rotations built
  once per size in `rotations()`), windowing and overlap.

The caller in `victory-hle::modules::audio` walks a voice's packets with
`packet`/`read_frame` and feeds `Decoder::decode`.

## Files

| Path | What it does | Key items |
|---|---|---|
| `crates/victory-formats/src/lib.rs` | Module list. Its doc still says BCPack and audio codecs are "planned"; both exist. | `bcpack`, `texture`, `xma` |
| `crates/victory-formats/src/texture.rs` | GFX10.3 texture sizes, mip placement, element addresses, tile/detile copies. | `Swizzle`, `Desc`, `Layout`, `Layout::new`, `Layout::offset`, `Layout::detile`, `Layout::tile`, `Layout::bytes_from`, `Layout::addressable`, `Mip`, `Tail`, `TextureDesc`, `Texture`, `Plane`, `format`, `MICRO`, `shared_pattern` |
| `crates/victory-formats/src/bcpack.rs` | BCPack decoder: rANS lanes, block records, BC1/3/4/5/6H/7 packing. | `decode`, `Tables`, `Tables::builtin`, `Tables::parse`, `TABLES_SIZE`, `Error`, `lanes`, `Lane::step`, `bc6`, `bc7`, `plain` |
| `crates/victory-formats/src/xma.rs` | XMA2 packet walk and WMA Pro frame decoder. | `packet`, `Packet`, `read_frame`, `Frame`, `Decoder`, `Decoder::decode`, `Decoder::flush`, `PACKET_BYTES`, `FRAME_SAMPLES`, `SUBFRAME_SAMPLES`, `imdct_half` |
| `crates/victory-formats/src/xma/tables.rs` | WMA Pro Huffman and level tables converted from FFmpeg's `wmaprodata.h`. | `CRITICAL_FREQ`, `SCALE`, `COEF0_*`, `COEF1*`, `VEC4_*`, `VEC2`, `VEC1` |
| `crates/victory-formats/examples/texture_layout_bench.rs` | Times `Texture::new` plus one `offset` for six layouts, in ns per layout. `cargo run --release -p victory-formats --example texture_layout_bench`. | `main` |

`crates/victory-formats/src/bcpack-tables.bin` is data, not source (see above).

## Switches

Only in ignored tests; none at runtime.

| Name | Effect | Read in |
|---|---|---|
| `VICTORY_BCPACK_TABLES` | table file for `reference_chunks` | `crates/victory-formats/src/bcpack.rs` |
| `VICTORY_BCPACK_CASES` | directory of `<name>.bc<n>` chunks with `<name>.out` beside them | `crates/victory-formats/src/bcpack.rs` |
| `VICTORY_XMA_STREAMS` | directory of `*.xma` with `*.ctx` context words, for `walks_saved_streams` | `crates/victory-formats/src/xma.rs` |
| `VICTORY_XMA_REFERENCE` | directory with `mono`/`stereo` `.xmadata` and `.ref` PCM, for `matches_a_reference_decoder` (SNR above 40 dB) | `crates/victory-formats/src/xma.rs` |

These tests are `#[ignore]` and **panic** (`expect`/`unwrap`) if the variable
is unset; they do not skip. Run them by name with `-- --ignored`. The
benchmarks `bcpack_throughput` and `imdct_throughput` are also ignored and use
synthetic data.

## Traps

- No game data in tests. Texture tests check addrlib's own sizes and an FNV-1a
  hash of all element addresses (`layouts_match_addrlib`); BCPack tests build
  synthetic tables in `tables()`; XMA tests use generated noise streams. The
  shipped tables are only checked to parse and for dictionary consistency.
- `MAX_EXTENT` also keeps all size arithmetic inside `u64`/`u32`; values come
  from titles unchecked. Keep the bound if you widen it.
- `dim` (copies) and `padded_dim` (placement) differ on purpose. Using one for
  the other gives wrong offsets for odd sizes.
- Mip order is smallest first. `bytes_from` and texture streaming rely on that.
- `walk` checks every buffer size before copying; `copy_run` itself indexes
  without further checks and panics if a check is missing.
- `addressable()` false means `Unsupported`, not a wrong address. Adding XOR
  modes needs the GPU's pipe and bank configuration, which differs between
  Series X and S.
- BCPack `decode` ignores bytes past what the lanes need and fails on a stream
  that runs out; `dst` must be a whole number of blocks.
- `Tables::parse` requires the exact `TABLES_SIZE`; the file format has no
  version.
- XMA `Decoder::decode` caps the bit count at the data length, so a too-large
  count cannot make it spin. It writes 512 samples per channel even on error.
- Licences: `texture.rs` carries addrlib's MIT notice; `xma.rs` and
  `xma/tables.rs` are derived from FFmpeg (LGPL-2.1-or-later, used under
  GPL-3.0-or-later). Keep those notices.

## Where to start for common changes

- A texture mode or format not handled: `Swizzle::block_log2`, the validity
  checks in `Layout::new`, `Layout::tiled`, `format`.
- Faster detiling: `Layout::walk`, `copy_runs`; measure with
  `texture_layout_bench` and the tests in the same file.
- BCPack speed: `lanes` and `Lane::step`; `bcpack_throughput` measures it.
- A wrong XMA frame boundary: `next_frame` and `Cursor`; the noise tests in
  `xma.rs` exercise packet edges.

## Origins

All three modules first came with Marvis' `2849477` (upstream VIctory).
Texture: addrlib layout test by Vyacheslav Venglovsky (`8f08131`), shared
swizzle tables and the bench by TG0000 (`8961e15`), MSAA linear rejected by
AlanQiang0418 (`379f4e7`), overlapping depth slices rejected by Pedro Fortunato
(`f4a7ae2`), row spans reused by ifBars (`fc807b6`), overflow guards by Vanthys
(`e158d95`), extent limit by 9hvn9yt2vk-netizen (`e6a2c54`), doc fix by Louis
Faust (`0f0c872`), fork changes by fxceless (`578d459`, `8dd30ab`); "a texture
of which only the smaller mips are there detiles from its start" is this
fork's `9526cb2`. BCPack: bucket window table by TG0000 (`3e982f7`), fork
changes by fxceless (`578d459`). XMA: zero-length frame stop by
9hvn9yt2vk-netizen (`005ad49`), continuation fix by tedin7 (`32f970c`),
rotations built once by TG0000 (`ade1da1`), noise tests by AnoHunted
(`717b3e8`), byte-wise payload gather by heromike405-arch (`ae0952d`), test
fixture checks by Vanthys (`4cca767`).
