//! The code maps in docs/agents/ stay true as the code changes.
//!
//! Four rules, each a set comparison:
//!
//! 1. every tracked `*.rs` file under `crates/`, `apps/`, `xtask/` or `tests/`
//!    is named, as its exact repo-relative path in backticks, in at least one
//!    `docs/agents/*.md`;
//! 2. every backticked `.rs` path under those roots in `docs/agents/*.md` is a
//!    tracked file (catches renames and deletions). Tokens containing `*`,
//!    `<`, `{` or `…` are patterns, not paths, and are skipped;
//! 3. every environment switch used in a tracked `*.rs`, `*.ps1` or `*.toml`
//!    file is named in docs/agents/switches.md;
//! 4. every switch named in docs/agents/switches.md is used in those files.
//!
//! A switch is `VICTORY_` followed by one or more of `[A-Z0-9_]`, taken as
//! the longest such run and kept exactly as written: a trailing `_` is part
//! of the token, so a prefix used to build names is documented as written,
//! prefix included, and never stands in for (or is satisfied by) a full
//! name. It must not touch an identifier character on either side
//! (`[A-Za-z0-9_]` before it, `[a-z]` after it), so it is not part of a
//! longer word. The same rule reads sources and switches.md.
//!
//! Files are listed as `cargo xtask policy` lists them (tracked plus
//! untracked-but-not-ignored); listed files missing from the working tree
//! are skipped.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Result, bail};

const SOURCE_ROOTS: &[&str] = &["crates/", "apps/", "xtask/", "tests/"];
const SWITCH_EXTENSIONS: &[&str] = &[".rs", ".ps1", ".toml"];
const MAPS_DIR: &str = "docs/agents/";
const SWITCHES_DOC: &str = "docs/agents/switches.md";
const PREFIX: &str = "VICTORY_";

pub fn run(root: &Path) -> Result<()> {
    let files: Vec<String> = crate::policy::list_files(root, false)?
        .into_iter()
        .filter(|p| root.join(p).is_file())
        .collect();
    let read = |p: &str| {
        String::from_utf8_lossy(&std::fs::read(root.join(p)).unwrap_or_default()).into_owned()
    };
    let maps: Vec<(String, String)> = files
        .iter()
        .filter(|p| is_map(p))
        .map(|p| (p.clone(), read(p)))
        .collect();
    let switch_sources: Vec<(String, String)> = files
        .iter()
        .filter(|p| SWITCH_EXTENSIONS.iter().any(|e| p.ends_with(e)))
        .map(|p| (p.clone(), read(p)))
        .collect();
    let switches_doc = maps
        .iter()
        .find(|(p, _)| p == SWITCHES_DOC)
        .map_or("", |(_, t)| t.as_str());

    let problems = check(&files, &maps, &switch_sources, switches_doc);
    if problems.is_empty() {
        println!(
            "agent docs: {} source files mapped, {} switches documented",
            files.iter().filter(|p| is_mapped_source(p)).count(),
            switch_tokens(switches_doc).len()
        );
        return Ok(());
    }
    for p in &problems {
        eprintln!("DOCS: {p}");
    }
    bail!(
        "{} code-map problem(s). Update docs/agents/*.md with the code: one row per \
         source file, every switch in docs/agents/switches.md",
        problems.len()
    )
}

/// All four rules over in-memory inputs. Pure, for testing.
pub fn check(
    files: &[String],
    maps: &[(String, String)],
    switch_sources: &[(String, String)],
    switches_doc: &str,
) -> Vec<String> {
    let tracked: BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let mut problems = unmapped_sources(&tracked, maps);
    problems.extend(stale_paths(&tracked, maps));
    let used = switch_uses(switch_sources);
    let documented = switch_tokens(switches_doc);
    problems.extend(undocumented_switches(&used, &documented));
    problems.extend(stale_switches(&used, &documented));
    problems
}

/// Rule 1.
fn unmapped_sources(tracked: &BTreeSet<&str>, maps: &[(String, String)]) -> Vec<String> {
    let named: BTreeSet<&str> = maps.iter().flat_map(|(_, t)| backticked(t)).collect();
    tracked
        .iter()
        .filter(|p| is_mapped_source(p) && !named.contains(*p))
        .map(|p| {
            format!("{p}: not named in any docs/agents/*.md (add a row to the map of its crate)")
        })
        .collect()
}

/// Rule 2.
fn stale_paths(tracked: &BTreeSet<&str>, maps: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (doc, text) in maps {
        let stale: BTreeSet<&str> = backticked(text)
            .filter(|t| is_path_token(t) && !tracked.contains(t))
            .collect();
        for t in stale {
            problems.push(format!(
                "{doc}: names `{t}`, which is not a tracked file (renamed or deleted? fix or drop the row)"
            ));
        }
    }
    problems
}

/// Rule 3.
fn undocumented_switches(
    used: &BTreeMap<String, String>,
    documented: &BTreeSet<String>,
) -> Vec<String> {
    used.iter()
        .filter(|(s, _)| !documented.contains(*s))
        .map(|(s, first)| {
            format!("{s}: used in {first} but not named in {SWITCHES_DOC} (add a row for it)")
        })
        .collect()
}

/// Rule 4.
fn stale_switches(used: &BTreeMap<String, String>, documented: &BTreeSet<String>) -> Vec<String> {
    documented
        .iter()
        .filter(|s| !used.contains_key(*s))
        .map(|s| {
            format!(
                "{SWITCHES_DOC}: names {s}, which no tracked .rs/.ps1/.toml file uses \
                 (drop the row, or rename it to match the code)"
            )
        })
        .collect()
}

/// Every switch in `sources`, with the first file (in list order) using it.
fn switch_uses(sources: &[(String, String)]) -> BTreeMap<String, String> {
    let mut used = BTreeMap::new();
    for (path, text) in sources {
        for s in switch_tokens(text) {
            used.entry(s).or_insert_with(|| path.clone());
        }
    }
    used
}

/// A map: directly in docs/agents/, Markdown.
fn is_map(path: &str) -> bool {
    path.strip_prefix(MAPS_DIR)
        .is_some_and(|rest| !rest.contains('/') && rest.ends_with(".md"))
}

/// A source file that must be named in a map.
fn is_mapped_source(path: &str) -> bool {
    SOURCE_ROOTS.iter().any(|r| path.starts_with(r)) && path.ends_with(".rs")
}

/// A backticked token that claims to be a source path (rule 2).
fn is_path_token(token: &str) -> bool {
    is_mapped_source(token) && !token.contains(['*', '<', '{', '…'])
}

/// Contents of the `code spans` on each line, trimmed. A line holds an even
/// number of backticks in well-formed Markdown; a stray one only shifts that
/// line.
fn backticked(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .flat_map(|line| line.split('`').skip(1).step_by(2))
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Every distinct switch token in `text` (see the module comment).
fn switch_tokens(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let is_tail = |b: u8| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_';
    let mut tokens = BTreeSet::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(PREFIX).map(|i| from + i) {
        let start_tail = at + PREFIX.len();
        let len = bytes[start_tail..]
            .iter()
            .take_while(|&&b| is_tail(b))
            .count();
        let end = start_tail + len;
        let clean_start = at == 0 || !is_ident(bytes[at - 1]);
        let clean_end = bytes.get(end).is_none_or(|b| !b.is_ascii_lowercase());
        if len > 0 && clean_start && clean_end {
            tokens.insert(text[at..end].to_owned());
        }
        from = end.max(at + 1);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keeps the maps true wherever tests run, even in clones without the hook.
    #[test]
    fn repository_docs_are_current() {
        super::run(&crate::workspace_root()).unwrap();
    }

    // Switch names are built at run time so this file names none itself.
    fn sw(tail: &str) -> String {
        format!("{PREFIX}{tail}")
    }

    fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn extracts_switch_tokens() {
        let text = format!(
            "env::var(\"{a}\"); {b}=1 {c}, x{d} {e}lower {f}{{n}} `{g}` {p}\"",
            a = sw("FOO"),
            b = sw("BAR_2"),
            c = sw("PREFIX_"),
            d = sw("GLUED"),
            e = sw("WORD"),
            f = sw("FMT_"),
            g = sw("TICK"),
            p = PREFIX,
        );
        let got: Vec<String> = switch_tokens(&text).into_iter().collect();
        assert_eq!(
            got,
            vec![
                sw("BAR_2"),
                sw("FMT_"),
                sw("FOO"),
                sw("PREFIX_"),
                sw("TICK")
            ]
        );
    }

    #[test]
    fn trailing_underscore_is_kept_on_both_sides() {
        let used = switch_uses(&owned(&[("a.rs", &format!("\"{}\"", sw("CARD_")))]));
        let documented = switch_tokens(&format!("| `{}` |", sw("CARD")));
        assert_eq!(undocumented_switches(&used, &documented).len(), 1);
        assert_eq!(stale_switches(&used, &documented).len(), 1);
    }

    #[test]
    fn extracts_backticked_tokens() {
        let md = "| `crates/a/src/b.rs` | does ` x ` and `y`\n```rust\nlet z;\n```\n";
        let got: Vec<&str> = backticked(md).collect();
        assert_eq!(got, vec!["crates/a/src/b.rs", "x", "y", "rust"]);
    }

    #[test]
    fn path_tokens() {
        assert!(is_path_token("crates/a/src/b.rs"));
        assert!(is_path_token("xtask/src/main.rs"));
        assert!(!is_path_token("crates/a/src/*.rs"));
        assert!(!is_path_token("crates/<name>/src/lib.rs"));
        assert!(!is_path_token("crates/a/src/{b,c}.rs"));
        assert!(!is_path_token("crates/a/src/….rs"));
        assert!(!is_path_token("crates/a/src/b.rs:12"));
        assert!(!is_path_token("docs/a.rs"));
        assert!(!is_path_token("crates/a/Cargo.toml"));
    }

    #[test]
    fn maps_are_markdown_directly_in_docs_agents() {
        assert!(is_map("docs/agents/victory-gpu.md"));
        assert!(is_map("docs/agents/switches.md"));
        assert!(!is_map("docs/agents/old/x.md"));
        assert!(!is_map("docs/agents/notes.txt"));
        assert!(!is_map("docs/asset-policy.md"));
    }

    #[test]
    fn finds_unmapped_and_stale_paths() {
        let files: Vec<String> = [
            "crates/a/src/lib.rs",
            "crates/a/src/new.rs",
            "apps/x/src/main.rs",
            "tests/fixtures/p/build.rs",
            "docs/agents/a.md",
            "README.md",
            "scripts/tool.rs",
        ]
        .map(String::from)
        .to_vec();
        let maps = owned(&[
            (
                "docs/agents/a.md",
                "| `crates/a/src/lib.rs` | lib |\n| `crates/a/src/gone.rs` | old |\n\
                 | `crates/a/src/*.rs` | all |\n| `apps/x/src/main.rs` | app |",
            ),
            (
                "docs/agents/b.md",
                "`tests/fixtures/p/build.rs` and `crates/a/src/lib.rs`",
            ),
        ]);
        let problems = check(&files, &maps, &[], "");
        assert_eq!(
            problems,
            vec![
                "crates/a/src/new.rs: not named in any docs/agents/*.md \
                 (add a row to the map of its crate)"
                    .to_owned(),
                "docs/agents/a.md: names `crates/a/src/gone.rs`, which is not a tracked file \
                 (renamed or deleted? fix or drop the row)"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn finds_undocumented_and_stale_switches() {
        let sources = owned(&[
            ("crates/a/src/lib.rs", &format!("var(\"{}\")", sw("ONE"))),
            (
                "run.ps1",
                &format!("$env:{} = 1; $env:{} = 1", sw("ONE"), sw("TWO")),
            ),
            ("victory.toml", &format!("# {}", sw("THREE"))),
        ]);
        let doc = format!(
            "| `{}` |\n| `{}` |\n| `{}` |",
            sw("ONE"),
            sw("THREE"),
            sw("OLD")
        );
        let problems = check(&[], &[], &sources, &doc);
        assert_eq!(
            problems,
            vec![
                format!(
                    "{}: used in run.ps1 but not named in {SWITCHES_DOC} (add a row for it)",
                    sw("TWO")
                ),
                format!(
                    "{SWITCHES_DOC}: names {}, which no tracked .rs/.ps1/.toml file uses \
                     (drop the row, or rename it to match the code)",
                    sw("OLD")
                ),
            ]
        );
    }

    #[test]
    fn a_switch_needs_its_whole_name_documented() {
        let sources = owned(&[("a.rs", &format!("{} {}", sw("CARD"), sw("CARD_ALL")))]);
        let doc = format!("`{}`", sw("CARD_ALL"));
        let problems = check(&[], &[], &sources, &doc);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with(&format!("{}:", sw("CARD"))));
    }
}
