# Provenance: who made what, and how credit is recorded

This map is for agents who change this repository. It records where the code
came from, who appears in the history, and the conventions this repository
uses to credit work taken from elsewhere. Everything here comes from `git log`,
`git shortlog`, `README.md` ("Credits") and `docs/README-fxceless.md`. Where
the history is ambiguous, this file says so.

**Rule for every agent:** the git history and every credit stay as they are.
Do not rewrite, squash, rebase or filter history. Do not remove or reword a
credit in a commit, the README or `docs/README-fxceless.md`. When you take work
from elsewhere, credit it in the commit subject as described in section 3.

Hashes are given short. Every hash called "ours" below was checked with
`git cat-file -t` and is a commit in this repository. Hashes of other
repositories (their commits, quoted from our messages) are **not** objects in
this repository. They are marked "theirs" and cannot be looked up here.

---

## 1. Lineage

The chain of repositories:

**VIctory (Marvisak) → fxceless/victory-fork → avatarmanz/VIctory-7900XTX →
Iupin/VIctory-7900XTX (this repository)**

The whole history is 621 commits (`git rev-list --all --count`), 191 of them
merges. Author dates run from 2026-11-23 01:15 +0100 to 2026-11-25 16:14 -0600.

### Stage 1: VIctory (github.com/Marvisak/VIctory)

- Begins at `f4504f6` "Initial commit" (Marvis, 2026-11-23 01:15 +0100).
- Ends at `ffe310e` "Shaders: the console's waterfall loop over a first-lane
  table offset" (Marvis, 2026-11-24 10:53 +0100). The next stage's first
  commit names this as its base: `578d459` "...: this fork's changes
  on top of VIctory main (ffe310e)".
- Shape: Marvis's own commits plus GitHub pull-request merges. 87 commits
  have committer "GitHub" and subject "Merge pull request #N from
  <user>/<branch>", e.g. `e8f3b1c` "Merge pull request #4 from
  Exorion1er/fix/tls-callbacks-reloc". The contributors' own commits sit
  on the merged side of each.
- Later VIctory work came in after stage 1 ended, in stage 3 (see "VIctory
  after stage 1" below).

### Stage 2: fxceless/victory-fork (github.com/fxceless/victory-fork)

- Begins at `578d459` (fxceless, 2026-11-24 13:02 -0500). Its body: "Based on
  VIctory by Marvisak and contributors (GPL-3.0-or-later); XCL by ivall and
  contributors was used as a reference." `docs/README-fxceless.md` says:
  "Upstream's history is kept intact below the single commit that carries this
  fork's changes", and `578d459` is that single commit.
- On the first-parent line of `main`, stage 2 runs from `578d459` to `4e17478`
  "The game is no longer named in the tree" (fxceless, 2026-11-24 15:45 -0500).
  Three squash-merged pull requests of that fork lie in between, committed
  by "GitHub": `d279e69` (ZazaJr24, "(#1)"), `3930989` (oddiums, "(#14)") and
  `90228a7` (cxccxc, "(#11)").
- fxceless's fork kept going after `4e17478`, with commits `9e49b8a` through
  `a26859f` (fxceless, last one 2026-11-25 01:46 -0500). Stage 3 took them in
  as merges, keeping their hashes: `fa967c0` "Merge fork main a803c87",
  `7913a98` "Merge fork main c96b4f2", `e5f735d` "Merge fork main 310c6eb",
  `49c44f6`/`ab71750` (fork main 8dd30ab), `925736d` "Merge fork main a26859f
  (the first room renders right)". 16 commits are by fxceless in all.
- `docs/README-fxceless.md` is that fork's README, kept with a short note at
  the top. Commit `3b30d6f` added it ("fxceless' README kept in docs").

### Stage 3: avatarmanz/VIctory-7900XTX

- Begins at `751c212` "Merge PR 24" (local, 2026-11-24 15:30 -0600). Its first
  parent is `4e17478`.
- Ends, as far as this history goes, at `0a43566` "README: where it is cloned
  from" (local, 2026-11-25 16:14 -0600). That commit changed the README's
  clone line from "git clone <this repository>" to
  `git clone https://github.com/avatarmanz/VIctory-7900XTX`.
- Almost all of stage 3's own commits are authored and committed as
  `local <local@localhost>`, with time zone -0600 (275 of the 276 "local"
  commits). The history does not say who is behind that identity. README
  Credits says "This fork's changes were made with Claude Code", and 220 of
  the "local" commits carry a `Co-Authored-By: Claude Opus 5.5` trailer.
- Branch names in merge subjects (`local/next`, `local/copies`, `night/cpu`,
  `day/shaders`, ...) are this stage's working branches. Their commits are
  in the history. The branches themselves are not in the repository.

### Stage 4: Iupin/VIctory-7900XTX (this repository)

- Per the owner, 2026-11-25: this repository is github.com/Iupin/VIctory-7900XTX,
  a fork of github.com/avatarmanz/VIctory-7900XTX that builds on it. The
  history alone does not show this step.
- So far the fork has no commits of its own beyond the original's: `main`,
  `origin/main` and `docs/agent-maps` all point at `0a43566`. The maps
  under `docs/agents/` were being written, not yet committed, when this file
  was made.

### Remote and clone URL

- `git remote -v`: `origin https://github.com/Iupin/VIctory-7900XTX.git` (fetch
  and push). Per the owner, this is the correct remote for this repository.
- `README.md` "how to run" gives `git clone https://github.com/avatarmanz/VIctory-7900XTX`,
  the URL of the original repository (stage 3). Both are recorded here as they
  are. Do not edit the README to change this unless the owner asks.

### VIctory after stage 1

Upstream VIctory commits made after `ffe310e` came into stage 3 in four ways:

- **Cherry-picks keeping the original author.** `925736d..daeeb28^` is 88
  commits, each with a "(cherry picked from commit <theirs>)" line. Then comes
  `daeeb28` "VIctory's 88 commits fitted in; clzero loops end as the console's
  do". Two more follow: `913d4f0` (quaternion) and `64c3178` (AnoHunted). In
  these commits the author is the original contributor (Manuel Pérez, Duff,
  Marvis, 给伊莉雅补魔, ...) and the committer is "local". 94 commits carry a
  "cherry picked from" line in all. None of the commits they name is in this
  repository.
- **By hand:** `b9c18b0` "VIctory e29391f, 13185f5 and 4fc9d70, by hand".
- **Ports named "upstream"** (code rewritten here, upstream hashes in the
  message): `4e3e065`, `75cfb23`, `011735f`, `c5a2f8d`, `77b82c9`, `26a801e`
  (merged by `3bc3455` "Merge local/shaders2: upstream translator and state
  fixes"), `1a93f0f`, `9fea743`, `b558934`, `9a6c829`. Section 4 lists them.
- **"Merge PR N" merges** in stage 3 (`751c212` and others, see section 4).
  These keep the contributors' commits as they were. Their messages do not
  name the repository the pull requests belonged to.

---

## 2. Who appears in the history

`git shortlog -sne --all`, names only. 99 name/e-mail identities, 621 commits.
"Marvis" and "ifBars" appear twice because each used two e-mail addresses.
There is no `.mailmap`.

| Name | Commits | Name | Commits | Name | Commits |
|---|---|---|---|---|---|
| local | 276 | Exorion | 2 | Lyrnc | 1 |
| Marvis | 87 | Henrique Bueno | 2 | Maledetto | 1 |
| Marvis | 17 | Jeremy Goldberg | 2 | Nixitov | 1 |
| Duff | 16 | KoLolX3 | 2 | Nostyxx | 1 |
| fxceless | 16 | Louis Faust | 2 | NuraadGit | 1 |
| Maraco91 | 8 | Pedro Fortunato | 2 | Omeya Sawant | 1 |
| Vyacheslav Venglovsky | 8 | Saitken91 | 2 | SlopComp | 1 |
| Manuel Pérez | 7 | Vanthys | 2 | Thomas Askharoun | 1 |
| TG0000 | 7 | Yaekai | 2 | Vax | 1 |
| marquezini | 7 | hophaver | 2 | ZazaJr24 | 1 |
| 给伊莉雅补魔 | 7 | lameloball2813-ui | 2 | alessiodeia | 1 |
| quaternion | 6 | nathanbzrr | 2 | artor410 | 1 |
| r4. | 6 | spacede | 2 | ashrafsdrop | 1 |
| 9hvn9yt2vk-netizen | 5 | Abdellah K. | 1 | broccolize | 1 |
| Enzo Souza Duarte | 5 | Advait Palve | 1 | bvtterbroot | 1 |
| FIocker | 5 | AgneyaK | 1 | cxccxc | 1 |
| fuzeofficial | 5 | Aws | 1 | divyeshganta | 1 |
| tedin7 | 5 | Bloxive | 1 | fluffyflattershy-afk | 1 |
| Claude | 4 | Calvix2k | 1 | heromike405-arch | 1 |
| EatCodeSleep2 | 4 | Carlos Sant'Anna | 1 | ifBars | 1 |
| LF2000 Builder | 4 | Codex | 1 | imjustprism | 1 |
| Luiz Neves | 4 | Dathinel | 1 | ismailchaka1 | 1 |
| Noname0202 | 4 | David Palacios | 1 | itsRevela | 1 |
| fever308 | 4 | Devin | 1 | julio | 1 |
| ifBars | 4 | Diego | 1 | lordon1a | 1 |
| AnoHunted | 3 | DocBlack89 | 1 | meower | 1 |
| BastienRdz | 3 | DoubleD17 | 1 | mgunes1311-lab | 1 |
| Tex | 3 | Eduardo Bertozzi | 1 | moe-miau | 1 |
| jaschinski97-ops | 3 | Giuseppe Rainone | 1 | nino-exe | 1 |
| raffclar | 3 | HailLucifer666 | 1 | oddiums | 1 |
| AlanQiang0418 | 2 | LUIS LARIOS | 1 | pes7 | 1 |
| Andrea Casale | 2 | LaunchDoesntCode | 1 | pgm1207 | 1 |
| BaggyALPHA | 2 | Luiz Otavio Mello de Campos | 1 | warmooff-ops | 1 |

Notes on the identities:

- **"local" is ambiguous.** Not every "local" commit belongs to stage 3. One,
  `f4438b3` (time zone -0500), came into stage 1 through VIctory's
  `ce42e77` "Merge pull request #432 from Proudnoob4/fix/uninitialized-sgprs".
  And `ca1d519` credits "ZJOU acf478b8, author local", so another repository's
  commit had the same author name. Do not read "local" as one person.
- Commits list GitHub user names as their authors' names, and these
  sometimes differ from the branch owner in the merge subject. Examples:
  "Exorion" in `Exorion1er/...`, "Duff" in `Dufftwt/...`, "r4." in `r4tedx/...`.
  Credit the name as git records it, and the merge subject as written.
- Committers: 366 commits have committer "local" and 91 have "GitHub"
  (pull-request merges and squash merges).

### Co-Authored-By trailers

Lines starting `Co-Authored-By:` or `Co-authored-by:` in commit bodies (324
commits carry at least one). Names only:

| Trailer name | Count | Commits that carry it are authored by (count) |
|---|---|---|
| Claude Opus 5.5 | 280 (277 `Co-Authored-By`, 3 `Co-authored-by`) | local 220, Vyacheslav Venglovsky 8, Marvis 7, TG0000 5, Manuel Pérez 5, LF2000 Builder 4, Claude 4, BastienRdz 3, and 1-2 each by 20 others |
| Claude Fable 5.2 | 37 | fxceless 16, Marvis 8, Maraco91 8, r4. 5 |
| Gemini 4 Neon | 3 | BastienRdz 3 |
| GPT 6.2 Sol | 2 | TG0000 2 |
| Devin AI | 2 | Louis Faust 2 |
| Claude Sonnet 5.5 | 1 | Marvis 1 |
| BlueKaiman | 1 | alessiodeia 1 |
| ZazaJr24 | 1 | ZazaJr24 1 (`d279e69`) |

Four commit bodies also carry a `Claude-Session:` line (e.g. `077ec8b`).

---

## 3. How credit is recorded in this repository

These conventions are in use. Follow them for new work.

1. **History is kept, never rewritten.** Other repositories' commits come in
   as merges, with their hashes kept. Examples: VIctory's "Merge pull request
   #N from ..." merges, "Merge fork main a26859f" (`925736d`), "Merge PR 220"
   (`1ae8f26`, seven commits by Vyacheslav Venglovsky kept as they were).
2. **Cherry-picks keep the original author.** The committer becomes "local",
   and git's "(cherry picked from commit <full hash>)" line stays in the body.
   Example: `9e1014d` (Manuel Pérez).
3. **A change rewritten or adapted here names the source in the subject or
   body**, giving the repository, their short hash and, where known, their author.
   Forms seen in subjects:
   - `(from Belobug 84ea9c12, romainhedouin)`: `c96525d`. Also `ab86ef7`
     `(from Belobug 92d353f8, romainhedouin)` and `b32fb99`
     `(from Belobug dd799aad, romainhedouin)`.
   - `(after ZJOU acf478b8, author local)`: `ca1d519`.
   - `(from fxceless f5c03f3b, pull request 999: Geckomacabre 895c0b8f, 04df4350)`:
     `23925a4`. This is a chain: the fork's commit, the pull request it came
     through, and that pull request's author and commits.
   - `after Belobug 18e57db8`: `a989668`. `(both after fxceless c36971b2)`: `17c4717`.
   - A bare reference: `(DexNom/VIctory 921d8d3b)` in `e77a7d9`,
     `(victory-fork a2537066)` in `51d3f4d`.
   - Upstream VIctory: `(upstream c430b1d, 100de0e)` in `4e3e065`.
     "Ports of 6f36cdf (...), 97c8783 (...)" in the body of `75cfb23`.
     "VIctory e29391f, 13185f5 and 4fc9d70, by hand" in `b9c18b0`.
   - In bodies: "The idea is fxceless/victory-fork's, commit f85101b6" (`b8b30d1`);
     "(the idea: fxceless/victory-fork 39731795)" (`37332f4`); "cpu_sets and
     keep_on_fast_cores are fxceless/victory-fork's (framelog.rs, commit
     0b0074e0)" (`96f2f32`); "From DexNom/VIctory (GPL-3.0-or-later) bc3cd4b0
     and 5f4c3df4:" (`810efe5`).
4. **A finding or a look that took nothing is recorded too.** `449830f`: "Belobug
   (76387c2d) found draws flickering because the records past a list's budget
   were not the same ones from frame to frame." `200a9cc`: "Belobug ae75a66a
   is this tree's own opt.rs ... 7263339c is a trace: nothing to take."
   `6c63dea` (r4.): "the case ifBars reported on upstream PR #566".
5. **fxceless's fork uses the same habit**, e.g. `31714de` "Take the
   fix/amd-view-heap-clamp branch of triippywhale's fork (as one commit)",
   `3bff182` "Take the test/fx-prs branch of asdaasdsaddf's fork (as one
   commit)", `c96b4f2` "The depth bounds test (#82 by Kaivyyy, as is)",
   `a26859f` "(pull request #305 by Flyander, after upstream VIctory
   86a5ed6)" and "the fixes of pull request #747 by AscilCH", and `8dd30ab`
   "code by myokex, victory-amd c6dc4d8, after pull request #115 by Spexivus,
   its correction by joroyt and a part of VIctory #1122 by tightfall", and
   "(Joshua Odom, 10ea38e)".
6. **README "Credits"** lists the repositories and people at the level of
   features. It says: "The history is kept, and commits taken from elsewhere
   name the commit and its author."
7. **`docs/README-fxceless.md` is kept as fxceless's fork had it**, under a
   three-line note that says so. Its own Credits section (VIctory, XCL,
   myokex, Spexivus, joroyt, tightfall, Joshua Odom) is part of what is kept.
   Do not edit it.
8. **AI assistance** is recorded with `Co-Authored-By:` trailers (section 2),
   and the README says so.

### "from" and "after"

The repository does not define the two words anywhere. Commit bodies suggest
a difference, but not a strict one:

- **"from"**: their change is taken and fitted into our code. `810efe5` ("From
  DexNom/VIctory ... bc3cd4b0 and 5f4c3df4") lists each piece it takes. `23925a4`
  and the three Belobug/romainhedouin commits use "from" in the same way.
- **"after"**: the same idea or behaviour, written here differently. `ca1d519`
  ("after ZJOU acf478b8") explains: "Theirs compares the handles, which a title
  may write other views behind; here the views themselves (resource, host
  texture, description) are compared." `b8b30d1` is similar: "The idea is
  fxceless/victory-fork's ... Here one line holds what all queues were given".
  But `8dd30ab` uses "after" for one fork adapting another fork's pull request.
- When code is kept unchanged, the wording says so directly ("as is" in
  `c96b4f2`; "carried here unchanged apart from one merge conflict" in
  `docs/README-fxceless.md`; "are fxceless/victory-fork's" in `96f2f32`).

For new work: write "from <repo> <their hash>, <author>" when you take their
code, and "after <repo> <their hash>" when you only follow their idea.

---

## 4. Work taken from elsewhere, by source

Commits after stage 1 whose messages name an outside repository, commit or
person. "Theirs" hashes are quoted as the message gives them and are not in
this repository. Subjects are shortened where marked "...".

| Source (repository) | Their commit(s) | Their author (as named) | Ours | Subject |
|---|---|---|---|---|
| Belobug/VIctory-Belobug-AMD | 84ea9c12 | romainhedouin | `c96525d` | A root constant buffer not 256-byte aligned is copied into scratch memory once ... (from Belobug 84ea9c12, romainhedouin) |
| Belobug | 92d353f8 | romainhedouin | `ab86ef7` | A pipeline a list binds again is not kept and named again (from Belobug 92d353f8, romainhedouin) |
| Belobug | dd799aad | romainhedouin | `b32fb99` | A range shaders write is declared once until the list next writes title memory (from Belobug dd799aad, romainhedouin) ... |
| Belobug | 18e57db8 | not named | `a989668` | Passes over the screen have pipeline workers and a wait of their own ...; after Belobug 18e57db8 |
| VIctory-Belobug-AMD | 06031237, 4f45ab88, cd9f78ce | beloooo | `82ca134` | A stand-in for the title's traced indirect light (VICTORY_AMBIENT, off) ... (body: "After beloooo's constant") |
| VIctory-Belobug-AMD | 72e748ea | beloooo | `30f46c9` | Structure builder: fences around its buffer atomics (VICTORY_BVH_FENCES=0: none) |
| Belobug | 76387c2d (a finding) | not named | `449830f` | The frame line says how many indirect records ran empty for want of their list's budget ... |
| Belobug | ae75a66a, 7263339c (looked at, nothing taken) | not named | `200a9cc` | Scalar buffer loads four dwords a call ... (body) |
| FQY55kai/ZJOU-VICTORY | acf478b8 | local | `ca1d519` | The same render targets are not set again (after ZJOU acf478b8, author local) ... |
| DexNom/VIctory | 921d8d3b | not named | `e77a7d9` | Ranges of memory that changed under texture views are joined before they are looked up (DexNom/VIctory 921d8d3b) |
| DexNom/VIctory | 921d8d3b (its cube part) | not named | `9a6c829` | Shaders: normalized buffer codes give the nearest float, cube gradients take DXIL's base-2 logarithm ... |
| DexNom/VIctory | bc3cd4b0, 5f4c3df4 | not named | `810efe5` | Structure builder and rays: dispatches counted in threads, empty slots never entered ... |
| myokex/victory-amd | bddfb4a5 | myokex | `9526cb2` | Formats: a texture of which only the smaller mips are there detiles from its start |
| fxceless/victory-fork, its pull request 999 | f5c03f3b; Geckomacabre's 895c0b8f, 04df4350 | Geckomacabre | `23925a4` | Textures shaders write that the title reads on the CPU are copied back into title memory ... |
| fxceless | c36971b2 | fxceless | `17c4717` | Title buffers' copies down to any size in the card's memory ... (both after fxceless c36971b2) |
| fxceless | c36971b2 (shader part), via our `53abe45` "wip wide" | fxceless | `200a9cc` | Scalar buffer loads four dwords a call (VICTORY_SHADER_OPT=127; off): fxceless c36971b2's shader part |
| fxceless/victory-fork | 39731795 (idea) | fxceless | `37332f4` | Parts of submissions prepared as one (VICTORY_JOIN_PARTS=8; off) ... |
| fxceless/victory-fork | 0b0074e0 (framelog.rs) | fxceless | `96f2f32` | The threads a frame waits for are kept on the fast cores ... |
| fxceless/victory-fork | f85101b6 (idea) | fxceless | `b8b30d1` | Submissions are prepared and sent by a thread of ours (VICTORY_ASYNC_SUBMIT=1, off) |
| victory-fork | f85101b6 | fxceless | `a9c2e99` | Draw records of 2^23 vertices or more and unfilled mesh records run empty |
| victory-fork | a2537066 | not named | `51d3f4d` | A texture is not copied onto itself; the process opts out of the efficiency mode (victory-fork a2537066) |
| fxceless/victory-fork main (merged) | a803c87, c96b4f2, 310c6eb, 8dd30ab, a26859f (these are in this repository) | fxceless | `fa967c0`, `7913a98`, `e5f735d`, `49c44f6`, `ab71750`, `925736d` | Merge fork main ... |
| VIctory upstream | c430b1d, 100de0e | not named | `4e3e065` | Typed buffer loads of 16-bit channels pack two per register ... (upstream c430b1d, 100de0e) |
| VIctory upstream | 6f36cdf, 97c8783, b956320, c1c3281, e6fe8ec, e34cb53, 691236d | not named | `75cfb23` | Upstream graphics state fixes: rectangle attributes, sampler bounds, A8 copies, depth footprints |
| VIctory upstream | e45bacd, 23fc8a5, 7dfa86f | not named | `011735f` | Anisotropic samplers with point mips keep them where the card has them (upstream ...) |
| VIctory upstream | 7d04943, 645391b, 40f30be and others listed in the body | not named | `c5a2f8d` | Upstream instruction fixes: 64-bit literals, exact normalized ends, flat offsets, rare forms |
| VIctory upstream | c0228d9 | not named | `77b82c9` | Mesh primitive exports: corner indices are nine bits, the edge flags dropped (upstream c0228d9) |
| VIctory upstream | 2286e03 | not named | `26a801e` | Immediate sampler slots are kept apart by comparison mode (upstream 2286e03) |
| VIctory upstream | 072fde0 (test) | not named | `1a93f0f` | Dynamic descriptor slots: the search goes past the cursor as far as it should |
| VIctory upstream | "upstream 1733" (helper lanes) | not named | `9fea743` | Streaming, copy queue, texture pools and a shader cache that survives rebuilds |
| VIctory upstream | 0f637570, 6d021fe3 | not named | `b558934` | Structure builder: root views as memory by address, windows of its own on the card ... |
| VIctory | da82e444, 4243a96a, a1a2077a | not named | `9a6c829` | (same commit as the DexNom row above) |
| VIctory | e29391f, 13185f5, 4fc9d70 | not named | `b9c18b0` | VIctory e29391f, 13185f5 and 4fc9d70, by hand |
| VIctory | 88 cherry-picks (original authors kept) | the commits' authors | `925736d..daeeb28^`, then `daeeb28` | VIctory's 88 commits fitted in; clzero loops end as the console's do |
| VIctory (not named in the message) | cherry-picks | quaternion; AnoHunted | `913d4f0`, `64c3178` | (cherry picked from commit ...) |
| not named | "Merge PR N" merges, contributors' commits kept | NuraadGit (PR 24), pgm1207 (51), SlopComp (58), Noname0202 (9, 25), jaschinski97-ops (40, 52), warmooff-ops (23), spacede (38), fever308 (42, 45), Vyacheslav Venglovsky (220) | `751c212`, `3489be5`, `fa06da4`, `75f1741`, `0e1e518`, `cc8d3f7`, `2d12b19`, `40a2f6a`, `675275a`, `1cbc278`, `7f166af`, `1ae8f26` | Merge PR N |

Work named by fxceless's fork in its own commits, now in this history:

| Source | Their commit / PR | Author (as named) | Ours (fxceless's commit) |
|---|---|---|---|
| triippywhale's fork, branch fix/amd-view-heap-clamp | branch, as one commit | triippywhale | `31714de` |
| asdaasdsaddf's fork, branch test/fx-prs | branch, as one commit | asdaasdsaddf | `3bff182` |
| pull request #82 | | Kaivyyy | `c96b4f2` |
| pull request #305, after upstream VIctory 86a5ed6 | | Flyander | `a26859f` |
| pull request #747 | | AscilCH | `a26859f` |
| myokex/victory-amd c6dc4d8, after pull request #115 | | myokex; Spexivus; joroyt (correction); tightfall (part of VIctory #1122) | `8dd30ab` |
| 10ea38e ("of the same fork family" per `docs/README-fxceless.md`) | | Joshua Odom | `8dd30ab` |
| XCL (reference only) | | ivall and contributors | `578d459` (body) |

Gaps between README Credits and the history:

- README Credits names **cxy0072019-sketch** alongside Geckomacabre for
  the texture read-back. No commit message in the history names
  cxy0072019-sketch.
- README Credits lists **FQY55kai/ZJOU-VICTORY** without saying what was
  taken. The only commit that names it is `ca1d519`.
- README Credits says Belobug gave "the ambient light idea, pipeline workers,
  and work by romainhedouin". The matching commits are `82ca134`, `a989668`,
  `c96525d`, `ab86ef7` and `b32fb99`.

---

## 5. Per-area origins

"Introduced" is the first commit that added the path (`git log --diff-filter=A
--reverse`). "Most changes" is `git log --format=%an -- <path> | sort | uniq -c`
on HEAD, top four. "Commits" is how many commits `git log -- <path>` lists.
Remember that "local" can be more than one person (section 2).

| Path | Introduced (stage) | Commits | Most changes by |
|---|---|---|---|
| `crates/victory-core` | `f4504f6` Marvis (1) | 9 | Marvis 4, Enzo Souza Duarte 2, nino-exe 1, fxceless 1 |
| `crates/victory-formats` | `f4504f6` Marvis (1) | 23 | Marvis 5, TG0000 3, fxceless 2, Vanthys 2 |
| `crates/victory-game` | `f4504f6` Marvis (1) | 4 | fxceless 2, Marvis 2 |
| `crates/victory-gpu` | `f4504f6` Marvis (1) | 331 | local 230, Marvis 30, fxceless 13, Vyacheslav Venglovsky 7 |
| `crates/victory-hle` | `f4504f6` Marvis (1) | 123 | local 44, Marvis 22, Duff 7, marquezini 6 |
| `crates/victory-loader` | `f4504f6` Marvis (1) | 51 | Marvis 16, local 8, ifBars 4, Duff 4 |
| `crates/victory-pe` | `f4504f6` Marvis (1) | 10 | Marvis 3, Manuel Pérez 2, 给伊莉雅补魔 1, tedin7 1 |
| `crates/victory-pipeline` | `f4504f6` Marvis (1) | 3 | Marvis 1, DocBlack89 1, Codex 1 |
| `crates/victory-shader` | `f4504f6` Marvis (1) | 137 | local 37, Marvis 33, fxceless 13, r4. 6 |
| `apps/victory` | `f4504f6` Marvis (1) | 55 | local 29, fxceless 6, Marvis 6, r4. 2 |
| `xtask` | `f4504f6` Marvis (1) | 4 | Marvis 2, local 1, LF2000 Builder 1 |
| `tests` | `f4504f6` Marvis (1) | 4 | Marvis 4 |
| `scripts` | `3b30d6f` local (3) | 1 | local 1 |
| `crates/victory-gpu/src/host.rs` | `754619f` Marvis (1) | 127 | local 103, fxceless 7, Marvis 7, Vyacheslav Venglovsky 2 |
| `crates/victory-gpu/src/shadow.rs` | `2849477` Marvis (1) | 75 | local 62, fxceless 8, Marvis 3, fever308 2 |
| `crates/victory-gpu/src/descriptor/host.rs` | `2849477` Marvis (1) | 65 | local 44, fxceless 9, Marvis 6, hophaver 2 |
| `crates/victory-gpu/src/resource.rs` | `754619f` Marvis (1) | 32 | local 19, fxceless 7, Marvis 3, fever308 2 |
| `crates/victory-gpu/src/draw.rs` | `2849477` Marvis (1) | 89 | local 66, fxceless 8, Marvis 6, fever308 2 |
| `crates/victory-gpu/src/watch.rs` | `2849477` Marvis (1) | 36 | local 29, fxceless 3, Marvis 2, Manuel Pérez 1 |
| `crates/victory-gpu/src/indirect.rs` | `8c26061` Marvis (1) | 34 | local 23, Vyacheslav Venglovsky 6, fxceless 3, Marvis 2 |
| `crates/victory-gpu/src/descriptor.rs` | `754619f` Marvis (1) | 34 | local 26, fxceless 4, Marvis 2, Manuel Pérez 1 |
| `crates/victory-gpu/src/list.rs` | `754619f` Marvis (1) | 39 | local 20, Marvis 7, fxceless 5, fuzeofficial 3 |
| `crates/victory-gpu/src/texture.rs` | `2849477` Marvis (1) | 24 | local 13, fxceless 7, Marvis 2, TG0000 1 |
| `crates/victory-gpu/src/pipeline/host.rs` | `2849477` Marvis (1) | 14 | local 6, fxceless 3, Marvis 3, pgm1207 1 |
| `crates/victory-gpu/src/tiles.rs` | `8dd30ab` fxceless (2) | 2 | fxceless 2 |
| `crates/victory-gpu/src/replay/play.rs` | `1deaf0e` local (3) | 15 | local 15 |
| `crates/victory-gpu/src/replay/record.rs` | `1deaf0e` local (3) | 6 | local 6 |

README Credits gives VIctory's share by feature: "the loader, the runtime
services, the Direct3D 12 back end and the shader recompiler."

---

## 6. Licence facts

- **GPL-3.0-or-later.** `LICENSE` is the GNU GPL version 3. The workspace
  `Cargo.toml` sets `license = "GPL-3.0-or-later"`, and the crates inherit it
  (`license.workspace = true`). README: "GPL-3.0-or-later, as VIctory."
  `578d459` and `docs/README-fxceless.md` say the same for VIctory and
  fxceless's fork. `810efe5` notes DexNom/VIctory is GPL-3.0-or-later.
- **The one exception:** `crates/victory-formats/src/bcpack-tables.bin`
  holds the BCPack format's constant tables. It was added in `2849477`
  (Marvis) and is entered as an exception in `xtask/src/policy.rs`.
  `docs/asset-policy.md` says it ships "by the project owner's decision
  (2026-11-23)" and that "the repository's licence does not cover them".
  Nothing else of this kind is accepted.
- **XCL** (github.com/ivall/xcl, GPL-2.0-only) was read for how the same
  title behaves under another layer. README: "no code of it is here". The
  fxceless README: "Fixes inspired by it were written anew in VIctory's code."
  Keep it that way: do not copy XCL code into this repository.
- The licences of the other forks named in section 4 (Belobug, myokex, ZJOU)
  are not stated in any commit message or file here. Only DexNom/VIctory's
  licence is stated (`810efe5`), and fxceless's (`578d459`).
