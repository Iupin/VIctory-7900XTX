# victory-game

## Purpose

The one crate that is specific to the title rather than to the platform: its
container formats, the table of builds we recognise, and (by design) the hooks
we place in its code. Everything else in the workspace is meant to work with
any Xbox Series title ([docs/architecture.md](../architecture.md)). What may be
stored here is bounded by [docs/asset-policy.md](../asset-policy.md): build
identities, per-build offsets under our own names, byte-pattern signatures and
small constants we write, never anything copied from the title.

Callers: `apps/victory/src/run.rs` (`identify`) and `apps/victory/src/shader.rs`
(`sgd5`, which `translate`, `shaders`, `shader-info` and `replay --retranslate`
reuse through `shader::sources`). Depends on `memchr` and `zlib-rs` only.

## How it works

### Build identities

`KnownBuild { label, blake3 }` pairs a label of our choosing with the BLAKE3
(hex) of the original, unmodified executable. `KNOWN_BUILDS` is the table and
is **empty**: the comment says it stays so until the executable is available.
`identify(blake3_hex)` looks a hash up case-insensitively.

`victory run` (`apps/victory/src/run.rs` `run`) prepares the executable with
`victory_loader::prepare`, takes `prepared.original_hash`, and prints either
`build <label> (<hash>)` or `build unknown (<hash>)`. Nothing else branches on
the result today: an unknown build runs the same way.

### Hooks (mechanism only)

The crate docs reserve this crate for hooks "located by per-build offsets or
byte-pattern signatures". No hook lives here at present; the crate has no
hook module. The intended shape, from the crate docs and the asset policy, is:
identify the build by its BLAKE3 (`identify`), then either use offsets recorded
for that build under names we pick, or search the image for a signature that
we wrote. Code that patches title memory at load time today is in
`victory-loader` (`victory_loader::write_sites::apply_asked`, called from
`apps/victory/src/run.rs`) and is not keyed by build; see the loader map.

### SGD5 shader files

`crates/victory-game/src/sgd5.rs` reads the engine's shader file. Each record
holds one shader container, raw-deflate packed. The header tables are not
decoded, so records are found by scanning:

- `is_sgd5(file)` is true if the 4-byte `MAGIC` appears anywhere, so it also
  answers for an archive that holds SGD5 data at some offset.
- `records(file)` runs `memchr::memmem::find_iter` for a fixed 4-byte marker
  (a constant we wrote, `MARKER`) and keeps each hit that `record` can
  cross-check: a 32-bit size `n` just before the marker must equal the one just
  after it, `n >= 12`, and the stated unpacked size must be at most
  `MAX_UNPACKED` (64 MiB). The packed bytes are `at + 12 .. at + n`.
- `Record` exposes `offset` (of the marker), `kind` (one byte before the
  sizes; meaning unknown, values listed in the doc comment) and `name`, the
  printable name that ends at the NUL before `kind`, found by `name_before`
  (it matches the length byte against the distance to the NUL; from 32
  characters on the length byte is itself printable, which is why it searches
  back from the start of the printable run).
- `Record::unpack` inflates with `zlib_rs::Inflate` (raw deflate, window 15),
  growing the output buffer up to the stated size, and returns `None` unless
  the stream ends exactly at that size. After filling the buffer it tries one
  more byte to make sure the stream really ended (an oversized stream is
  rejected). A stall (no progress) or `BufError` also yields `None`.

`apps/victory/src/shader.rs` `sources` combines these packed records with every
raw container found by its magic, so the same file can yield both.

## Files

| Path | What it does | Key items |
|---|---|---|
| `crates/victory-game/src/lib.rs` | Crate scope, the known-build table and lookup. | `KnownBuild`, `KNOWN_BUILDS` (empty), `identify` |
| `crates/victory-game/src/sgd5.rs` | Finds and unpacks the shader records of SGD5 files by marker scan. | `MAGIC`, `is_sgd5`, `records`, `Record`, `Record::unpack`, `MAX_UNPACKED` |

## Switches

None.

## Traps

- Asset policy: a build entry holds only our label and the BLAKE3; a hook holds
  only an offset under our name or a signature we wrote. Never paste bytes,
  strings or disassembly from the executable into this crate or its tests. The
  SGD5 tests build their records with `record_bytes` from synthetic payloads.
- `records` is a heuristic. Any 4-byte coincidence with the marker that also
  passes the size cross-check is taken as a record; `unpack` returning `None`
  is the second filter. `shader.rs` reports such cases as "corrupt record".
- `record` indexes `file[at - 5]` and `file[nul]` after `at.checked_sub(6)`
  succeeded, so a marker within the first six bytes is skipped, not a panic.
- `MAX_UNPACKED` bounds allocation from untrusted sizes; keep a bound if you
  change it.
- `identify` result is informational only. Code that starts depending on it
  must handle `None` (every build is unknown while `KNOWN_BUILDS` is empty).

## Where to start for common changes

- Recognise a build: add a `KnownBuild` to `KNOWN_BUILDS` in
  `crates/victory-game/src/lib.rs` (label and BLAKE3 only).
- Add a hook: a new module here, keyed by `identify` or by a signature search,
  called from the load path in `apps/victory/src/run.rs` before `spawn_main`.
- Decode more of SGD5 (its header tables): `crates/victory-game/src/sgd5.rs`;
  `apps/victory/src/shader.rs` `Source` is what consumers see.

## Origins

`lib.rs` by Marvis (`f4504f6`, upstream VIctory); the first `sgd5.rs` by
Marvis (`754619f`). The
current `sgd5.rs` arrived with fxceless' `3bff182`, which takes the
test/fx-prs branch of asdaasdsaddf's fork as one commit.
