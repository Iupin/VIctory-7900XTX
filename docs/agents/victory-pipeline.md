# victory-pipeline

## Purpose

Offline conversion of the user's data into host formats, behind a
content-addressed on-disk cache. It defines the `Converter` trait (bytes in,
bytes out, deterministic) and `Cache`, which stores each result under the
BLAKE3 of converter id, version and input. Today its only user is the shader
cache in `crates/victory-gpu/src/shader_cache.rs` (converter id `shader.dxil`);
no texture or audio converter exists yet. It depends on `victory-core` for the
policy guard.

## How it works

`Converter` (`crates/victory-pipeline/src/lib.rs`) has three methods:
`id() -> &'static str` (stable, part of the key), `version() -> u32` (bump when
output for the same input changes), `convert(&[u8]) -> anyhow::Result<Vec<u8>>`.
It is `Send + Sync`.

`CacheKey::new(converter, input)` (`crates/victory-pipeline/src/cache.rs`)
hashes, in order: the id bytes, one `0` byte, the version as 4 little-endian
bytes, then the input. `to_hex` gives the 64-digit hex.

`Cache::open(root)` calls `victory_core::ensure_outside_repo(root)` and
`create_dir_all`. `Cache::path_for(key)` is `<root>/<first 2 hex>/<other 62>`.

`Cache::get_or_convert(converter, input) -> PathBuf`:

1. Compute the key and path. If `path.is_file()`, return it (a "cache hit",
   logged at trace level). The file is not read or verified.
2. Otherwise run `convert`. Errors get the context `converter <id> v<version>`.
3. Create the two-digit directory, open a staging file with `tempfile_in`, write
   the output, drop the handle, then `std::fs::rename` it onto the final path.
   Readers therefore never see a partial file.
4. If the write or the rename fails, the staging file is removed and the error
   returned.

`create_tempfile_in(dir, counter)` names staging files
`.tmp-<pid>-<n>` with a process-wide `AtomicU64` counter, opened with
`create_new`. On `AlreadyExists` it tries the next number, up to 128 attempts,
and never touches the existing file (a stale one from an interrupted run, or a
reused process id). Other I/O errors return at once.

Concurrency: there is no lock. Two threads or processes that miss on the same
key both convert and both rename; the last rename wins, and both outputs are
equal if the converter is deterministic. `Cache` is `Clone` and holds only the
root path.

How the shader cache uses it (read in `crates/victory-gpu/src/shader_cache.rs`,
described here only for orientation): the input is a 32-byte build fingerprint
followed by the container, `version()` is fixed at 1 because the fingerprint
does the versioning, and an entry that does not decode is deleted and
converted once more. See that crate's map for the fingerprint.

## Files

| Path | What it does | Key items |
|---|---|---|
| `crates/victory-pipeline/src/lib.rs` | Crate docs and the converter trait. | `Converter` |
| `crates/victory-pipeline/src/cache.rs` | Content-addressed store with write-then-rename publication. | `Cache`, `Cache::open`, `Cache::get_or_convert`, `Cache::path_for`, `CacheKey`, `create_tempfile_in` |

## Switches

None.

## Traps

- A hit is "the file exists". A truncated or foreign file at the right path is
  served as is; callers that care must validate and delete (the shader cache
  does).
- `version()` is the only invalidation besides the input bytes. Changing a
  converter's output without bumping it serves stale results forever.
- The id is hashed without a length prefix but followed by a `0` byte, so ids
  must not contain NUL.
- Staging files live in the same two-digit directory as the target so the
  rename stays on one volume. Moving them elsewhere would break atomicity.
- On Windows `rename` onto an existing file replaces it; the concurrent-miss
  case relies on that (not tested here).
- `Cache::open` refuses any root inside a tree with `.victory-root`
  (`refuses_cache_inside_repo` test).

## Where to start for common changes

- New offline conversion (textures, audio): implement `Converter` next to the
  format code that does the work, give it a fresh id, and call
  `Cache::get_or_convert` with a cache opened on `Config::cache_subdir(...)`.
- Change the on-disk layout or key: `CacheKey::new` and `Cache::path_for`.
  Every existing entry becomes unreachable (not deleted).

## Origins

Initial crate by Marvis (`f4504f6`, upstream VIctory). Staging collisions
retried by DocBlack89 (`0aa61ef`); staging file removed on a failed publish by
"Codex" (`7d73e2b`).
