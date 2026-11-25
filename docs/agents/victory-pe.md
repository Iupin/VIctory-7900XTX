# victory-pe

## Purpose

Read-only analysis of PE32+ (x86-64) images and of COFF import libraries
(`.lib`). It never modifies bytes. `victory-loader` parses the title with it
and patches the copy itself, using the raw header offsets from `layout`.
`victory-hle` and the loader use its `ImportSymbol` as the key of every
import. The CLI (`apps/victory`: `inspect`, `census`, `lib-exports`) prints
what it finds. Where imports go once parsed is in
[docs/architecture.md, "Import resolution"](../architecture.md#import-resolution).

## How it works

### Images: `ImageInfo::parse`

`crates/victory-pe/src/image.rs`, `ImageInfo::parse(data)` does one pass over
the file bytes. It uses the `object` crate (`PeFile64`) and checks a few
things itself:

1. The file must be `FileKind::Pe64` with machine `MACHINE_AMD64`. Anything
   else is `PeError::Unsupported`. Truncation and inconsistency are
   `PeError::Malformed` (`object` errors convert into it).
2. Sections become `SectionInfo` (name, rva, virtual size, file offset, file
   size, characteristics). Each section's raw data must end inside the file.
   A section with no raw data is allowed.
3. Imports: `parse_imports` walks the import descriptors. It prefers the
   unbound name table (`OriginalFirstThunk`) and falls back to the IAT for old
   linkers. `entries` pairs each name-table entry with its 8-byte IAT slot
   (`ImportEntry::iat_slot_rva`). Every slot is checked to lie inside
   `size_of_image`, because `object` does not check the IAT RVA.
4. Delay-load imports: `parse_delay_imports` does the same for the delay-load
   table, giving `ImageInfo::delay_imports`. The loader binds these eagerly.
5. Exports: `parse_exports` uses `object`'s generic export iterator, not the
   PE-specific table. The PE table keeps one name per address slot, so two
   names that share an ordinal would lose all but one. Forwarded exports have
   `rva: None` and `forward: Some("dll.name")` or `"dll.#n"`. Unused address
   slots (address 0) are not reported (test `unused_export_slots_are_not_exports`).
6. TLS: `parse_tls` reads `AddressOfCallBacks` from the TLS directory and
   walks the callback array (at most `MAX_TLS_CALLBACKS` = 1024) until a zero.
   It turns each VA into an RVA against the preferred base, giving
   `TlsInfo { directory_rva, callbacks_va, callback_rvas }`.
7. CodeView: `CodeViewInfo { pdb_path, guid, age }` from the debug directory.
   `CodeViewInfo::symbol_key` gives the symbol-server key (GUID in Windows
   byte order, then age).
8. Flags: `has_relocations` (a base-relocation directory, and relocations not
   stripped), `has_exception_table` (`.pdata`), `has_load_config`.

Helpers on `ImageInfo`: `is_dll`, `rva_to_file_offset` (uses
`min(virtual_size, file_size)` so padding past the virtual size is not
mapped, and uses `checked_add` against a hostile offset near 4 GiB),
`import_count`, `subsystem_name` (includes the Xbox subsystems 14 and 17).

### Raw offsets: `layout`

`crates/victory-pe/src/layout.rs` holds the PE/COFF field offsets that
`victory-loader::prepare` writes to: `FILE_HEADER_CHARACTERISTICS`,
`OPT_ADDRESS_OF_ENTRY_POINT`, `OPT_CHECKSUM`, `OPT_DLL_CHARACTERISTICS`,
`OPT_DATA_DIRECTORY`, `TLS64_ADDRESS_OF_CALLBACKS`, the `dir::*` data
directory indices, `file_flags`, `dll_flags` and `subsystem` values. It also
has bounds-checked readers (`read_u32`, `read_u64`) and offset finders
(`file_header_offset`, `optional_header_offset`, `data_directory_offset`,
which honours `NumberOfRvaAndSizes`). The offset functions are plain
arithmetic: an `e_lfanew` past the end is returned as is, and every read
through it fails instead.

### Import libraries: `implib`

`crates/victory-pe/src/implib.rs`, `ImportLibrary::parse(data)` reads a COFF
archive. Each member that is a short-format import object becomes a
`LibImport { dll, export: ImportSymbol, kind: LibImportKind::{Code, Data,
Const} }`. Other members (regular objects, long-format descriptors) are only
counted in `other_members`, so nothing is lost silently. Linker members (`/`,
`//`) are skipped by `object` and not counted. The list is sorted and
de-duplicated. The same symbol of a different kind stays as two entries.
Name cleanup is `object`'s: `NAME_NO_PREFIX` and `NAME_UNDECORATE` give the
plain export name, and `NAME_EXPORTAS` gives the export-as name. The hint of
a short-format import is not exposed by `object`, so it is always 0.

What it is used for: the module docs say an umbrella import library of the
console SDK maps each API to the DLL that provides it, which tells which of
our implementations answers which import. In the code today the only caller
is the CLI command `victory lib-exports <lib> [--list]`
(`apps/victory/src/inspect.rs`, `lib_exports`). It prints exports per DLL.
Nothing in the loader or `victory-hle` reads import libraries at run time;
the registry in `victory-hle` is written by hand.

## Files

| Path | What it does | Key items |
|------|--------------|-----------|
| `crates/victory-pe/src/lib.rs` | Crate root: re-exports the image types, declares `PeError` and the AMD64 machine constant. | `PeError::{Unsupported, Malformed}`, `MACHINE_AMD64` |
| `crates/victory-pe/src/image.rs` | Parses a whole PE32+ image in one pass: sections, imports, delay imports, exports, TLS callbacks, CodeView. Unit tests parse the test binary itself and synthetic images. | `ImageInfo::parse`, `ImageInfo::rva_to_file_offset`, `ImportModule`, `ImportEntry::iat_slot_rva`, `ImportSymbol`, `ExportInfo`, `TlsInfo`, `CodeViewInfo::symbol_key` |
| `crates/victory-pe/src/layout.rs` | Raw byte offsets and flag values of PE header fields, plus bounds-checked readers, for code that patches images. | `data_directory_offset`, `optional_header_offset`, `read_u32`, `read_u64`, `dir`, `dll_flags`, `file_flags`, `subsystem` |
| `crates/victory-pe/src/implib.rs` | Reads COFF import libraries into `dll!export` pairs. | `ImportLibrary::parse`, `LibImport`, `LibImportKind`, `other_members` |

## Switches

None. This crate reads no `VICTORY_...` variables.

## Traps

- `ImportSymbol` is the shared key. `Display` prints a name, or `#n` for an
  ordinal. `victory-hle`'s registry looks ordinals up by that string (`"#1"`),
  so changing `Display` breaks ordinal binding.
- `ImportSymbol::Name::hint` takes part in `Eq`, `Ord` and `Hash`. Import
  libraries always give hint 0 and images give the real hint. Comparing a
  `LibImport::export` with an image's `ImportEntry::symbol` by equality
  therefore fails when the hints differ; compare names instead.
- `ImageInfo` is parsed from the original bytes. The loader keeps that copy
  (`PreparedImage::info`) after patching. Re-parsing the patched bytes gives
  an image with no imports, no entry point and no TLS callbacks.
- The IAT bounds check in `entries` exists because `object` does not check
  it. An IAT that runs outside the image is `Malformed` (test
  `rejects_iat_outside_image`). Keep it if the parser is changed.
- Exports come from the generic iterator on purpose (see step 5 above).
  Switching to the PE-specific table loses names that share an ordinal (test
  `every_name_of_a_shared_ordinal_survives`).
- TLS callback VAs are converted against the preferred `image_base`, not the
  mapped base. Callers add the RVA to the real base.
- `layout`'s constants are fixed by `constants_match_the_pe32_layout`, which
  also pins the directory indices `prepare` patches by number.

## Origins

Initial code by Marvis (`f4504f6`, "Initial commit"). Later fixes, as the
commit log names them: Manuel Pérez (`e98d3ca` checked offsets in
`rva_to_file_offset`, `ef22723` layout offset checks), ifBars (`4da3d63`),
tedin7 (`9a5cbbe` unused export slots), Diego (`8b4f50f` section bounds),
给伊莉雅补魔 (`ed07828` synthetic tests for `implib.rs` and `layout.rs`),
quaternion (`98b36e1` every name of a shared ordinal).

## Where to start for common changes

- A new header field the loader must patch: add its offset to
  `crates/victory-pe/src/layout.rs`, add it to `constants_match_the_pe32_layout`,
  then use it from `crates/victory-loader/src/prepare.rs`.
- A new piece of image information (another data directory): extend
  `ImageInfo` and `ImageInfo::parse` in `crates/victory-pe/src/image.rs`.
  Check every RVA against `size_of_image` or the sections yourself; `object`
  does not always do it.
- An image that fails to parse: run `victory inspect` on it (it calls
  `ImageInfo::parse`) and look at the `PeError` text.
- Reading an SDK import library: `victory lib-exports <lib> --list`, or
  `ImportLibrary::parse` in `crates/victory-pe/src/implib.rs`.
