# For agents working in this repository

VIctory runs an Xbox Series title natively on Windows from files the user
supplies. This fork is about speed on an AMD RX 7900 XTX. `README.md` says where
it stands; this file says how to work here and where to look.

## Three rules that come before everything else

### 1. Nothing from the game is in the repository

`docs/asset-policy.md` is rule #1 and binds code, comments, tests, docs and
commit messages: no assets, no part of the executable, nothing derived from
either (strings, disassembly, shader dumps, screenshots, logs with game data).
The user's dump and all caches live outside the repository; tools refuse to
write below a directory holding `.victory-root`. Do not copy from the dump, and
do not open it unless the task needs it.

### 2. The history is kept as it is

This is a fork of a fork with work from many people (`docs/agents/provenance.md`).
The history is the record of who did what.

- Never rewrite published history: no rebase, squash, amend, `filter-repo` or
  force-push of commits that exist on a remote. Add new commits.
- Bring branches together with merge commits (`git merge --no-ff`), never by
  squashing, so the commits and their authors stay.
- Take other people's commits with `git cherry-pick -x` so author and source
  stay recorded.
- The one exception is the asset policy's own ("If something slips through"):
  that needs the owner's explicit instruction each time. Do not start it.
- Do not push unless asked.

### 3. Credit is never removed, and new borrowings are credited

- Do not delete or reword existing credits: the Credits section of `README.md`,
  `docs/README-fxceless.md` (kept verbatim), `LICENSE`, credit lines in source
  comments, `Co-Authored-By` trailers.
- When a change takes code or an idea from another repository or person, the
  commit subject says so in the form already in use here:
  `... (from <repository> <their short hash>, <their author>)` when their change
  was taken and fitted in, `... (after <repository> <hash>, <author>)` when the
  idea was written anew. A source new to this tree is also added to the README
  Credits list and to `docs/agents/provenance.md`.
- Code from elsewhere must be compatible with GPL-3.0-or-later. If its licence
  is not stated, ask the owner before taking it.

## Build, check, test

Windows x64, the toolchain pinned in `rust-toolchain.toml`, MSVC build tools.

```powershell
cargo build --locked --profile diag -p victory --features diagnostics   # the build that is run
cargo clippy --workspace --all-targets --locked -- -D warnings          # CI
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings   # CI
cargo test --workspace --locked                                         # CI; includes the two checks below
cargo xtask policy          # asset policy over the working tree
cargo xtask docs            # the code maps agree with the code
cargo xtask install-hooks   # once per clone: both checks before every commit
```

After a change to the shader translator (`crates/victory-shader`, or anything in
the shader cache key), a new build that changes pipelines, or a new GPU driver,
warm up before running or measuring, or the first minutes are spent making
pipelines (seconds a frame, thousands of draws dropped):

```powershell
target\diag\victory.exe shaders <the shader archives>   # every shader translated again, minutes
target\diag\victory.exe warm --used                     # the pipelines earlier runs drew with
```

Both can be stopped and go on later. A measurement counts only once the frame
line says `0 being made`.

Details, and what CI does not cover: `docs/agents/tooling.md`. Running the
title: `README.md` and `docs/agents/app-victory.md`. Whether a change made the
picture or the frame time better can only be judged on the owner's machine with
the owner's dump; say what you could and could not verify.

## The code maps

Read the map of the area before changing it; it says how the area works, what
each file is for, and what breaks easily.

| Area | Map |
|------|-----|
| `crates/victory-gpu`: host device, command lists, queues, submission, descriptors | `docs/agents/victory-gpu-host.md` |
| `crates/victory-gpu`: title memory, shadows, write tracking, textures | `docs/agents/victory-gpu-memory.md` |
| `crates/victory-gpu`: draws, indirect, pipelines, capture, replay, timing | `docs/agents/victory-gpu-draw.md` |
| `crates/victory-shader`: Xbox shader containers to host shaders | `docs/agents/victory-shader.md` |
| `crates/victory-loader`: prepare, map, bind and start the title image | `docs/agents/victory-loader.md` |
| `crates/victory-hle`: console API implementations | `docs/agents/victory-hle.md` |
| `crates/victory-pe`: PE and import-library analysis | `docs/agents/victory-pe.md` |
| `crates/victory-core`: config, asset-policy guard, logging | `docs/agents/victory-core.md` |
| `crates/victory-formats`: texture layouts, BCPack, audio codecs | `docs/agents/victory-formats.md` |
| `crates/victory-pipeline`: converter trait, content-addressed cache | `docs/agents/victory-pipeline.md` |
| `crates/victory-game`: title-specific parts | `docs/agents/victory-game.md` |
| `apps/victory`: the command-line front end, `scripts/play.ps1` | `docs/agents/app-victory.md` |
| `xtask`, CI, hooks, lints, toolchain | `docs/agents/tooling.md` |
| Every `VICTORY_...` environment switch | `docs/agents/switches.md` |
| Who made what, and how credit is recorded | `docs/agents/provenance.md` |

Written for people, and still true: `docs/architecture.md` (crates, import
resolution, shaders), `docs/findings.md` (what was measured and why things are
as they are), `docs/replay.md`, `docs/fork-ray-tracing.md`.

## Keeping the maps true

The maps are part of the change, not a follow-up. In the same commit as the code:

- a new, renamed or deleted `.rs` file: add, fix or drop its row in the map of
  its area (the path repo-relative, in backticks, exactly);
- a new, renamed or removed `VICTORY_...` switch: its row in
  `docs/agents/switches.md`, and the local mention in the area's map;
- a change to how something works (a flow, a thread, a lock order, a default):
  the prose of the map that describes it;
- something learned the hard way: a line under the map's "Traps".

`cargo xtask docs` enforces the first two and fails the commit hook and CI
otherwise. The third and fourth it cannot see; they are on you. If you find a
map wrong, fix it in the same change that found it. Where a source comment and
the code disagree, the map follows the code; several comments in
`crates/victory-gpu` still say "off by default" of things that are now on.

A switch that changes what the shader translator emits must also be entered in
the shader cache key by hand (`crates/victory-gpu/src/shader_cache.rs`); see
`docs/agents/victory-shader.md`, "Traps".

## How commits are written here

One subject line that says in plain words what now happens, with the switch
that turns it off or on in parentheses and the credit where there is one, for
example: `A pipeline a list binds again is not kept and named again (from
Belobug 92d353f8, romainhedouin)`. Work on a branch, not on `main`. Commit when
asked.
