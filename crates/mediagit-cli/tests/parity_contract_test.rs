// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! GUI parity contract: every CLI command and argument must be classified in
//! `ui/parity.toml` as surfaced (`gui = "View"`), not yet surfaced
//! (`gui = "TODO:View"`) or deliberately never surfaced (`excluded = "why"`).
//!
//! The clap tree is read from the built binary's hidden `__cli-tree` command
//! (the `Cli` struct lives in `main.rs`, so it is not importable). A new flag or
//! subcommand with no entry fails this test; so does an entry the CLI no longer
//! has. Run with `-- --nocapture` to see the TODO count.
//!
//! Run: `cargo test -p mediagit-cli --test parity_contract_test`

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Key for one argument of the clap tree JSON: `--long`, else `-s`, else the
/// positional's value name. `None` for clap's own help/version.
fn arg_name(a: &serde_json::Value) -> Option<String> {
    let id = a["id"].as_str()?;
    if matches!(id, "help" | "version") {
        return None;
    }
    if let Some(l) = a["long"].as_str() {
        return Some(format!("--{l}"));
    }
    if let Some(s) = a["short"].as_str() {
        return Some(format!("-{s}"));
    }
    Some(
        a["value_name"]
            .as_str()
            .map_or_else(|| id.to_uppercase(), str::to_string),
    )
}

fn tree() -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_mediagit"))
        .arg("__cli-tree")
        .output()
        .expect("spawn mediagit");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("__cli-tree prints JSON")
}

/// Every (command, argument) the CLI exposes. Commands are keyed by their path
/// (`"branch switch"`); arguments by `"<path> <arg>"`, or bare `"--machine"`
/// for the global flags declared on the root.
fn surface(root: &serde_json::Value) -> (BTreeSet<String>, BTreeSet<String>) {
    let names = |n: &serde_json::Value| -> Vec<String> {
        n["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(arg_name)
            .collect()
    };
    let globals: BTreeSet<String> = names(root).into_iter().collect();
    let mut cmds = BTreeSet::new();
    let mut args = globals.clone();
    let mut stack: Vec<(String, &serde_json::Value)> = root["subcommands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["name"].as_str().unwrap().to_string(), n))
        .collect();
    while let Some((key, node)) = stack.pop() {
        cmds.insert(key.clone());
        for a in names(node).into_iter().filter(|a| !globals.contains(a)) {
            args.insert(format!("{key} {a}"));
        }
        for s in node["subcommands"].as_array().unwrap() {
            if s["name"] != "help" {
                stack.push((format!("{key} {}", s["name"].as_str().unwrap()), s));
            }
        }
    }
    (cmds, args)
}

/// One `parity.toml` entry: `Ok(gui)` or `Err(excluded reason)`.
type Entry = Result<String, String>;

fn load_table(
    doc: &toml::Table,
    name: &str,
    problems: &mut Vec<String>,
) -> BTreeMap<String, Entry> {
    let mut out = BTreeMap::new();
    let Some(t) = doc.get(name).and_then(|v| v.as_table()) else {
        problems.push(format!("[{name}] table is missing"));
        return out;
    };
    for (key, v) in t {
        let gui = v.get("gui").and_then(|x| x.as_str());
        let excluded = v.get("excluded").and_then(|x| x.as_str());
        match (gui, excluded) {
            (Some(g), None) if !g.trim().is_empty() => {
                out.insert(key.clone(), Ok(g.to_string()));
            }
            (None, Some(r)) if !r.trim().is_empty() => {
                out.insert(key.clone(), Err(r.to_string()));
            }
            _ => problems.push(format!(
                "[{name}.\"{key}\"] needs exactly one non-empty `gui` or `excluded`"
            )),
        }
    }
    out
}

/// What is wrong between the CLI surface and the classification.
#[derive(Debug, Default, PartialEq)]
struct Drift {
    unclassified: Vec<String>,
    stale: Vec<String>,
}

fn diff(cli: &BTreeSet<String>, classified: &BTreeMap<String, Entry>, kind: &str) -> Drift {
    Drift {
        unclassified: cli
            .iter()
            .filter(|k| !classified.contains_key(*k))
            .map(|k| format!("{kind} {k}"))
            .collect(),
        stale: classified
            .keys()
            .filter(|k| !cli.contains(*k))
            .map(|k| format!("{kind} {k}"))
            .collect(),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn every_cli_command_and_argument_is_classified_for_the_gui() {
    let text = std::fs::read_to_string(repo_root().join("ui/parity.toml")).expect("ui/parity.toml");
    let doc: toml::Table = text.parse().expect("ui/parity.toml is valid TOML");
    let mut problems = Vec::new();
    let cmds = load_table(&doc, "cmd", &mut problems);
    let args = load_table(&doc, "arg", &mut problems);

    let (cli_cmds, cli_args) = surface(&tree());
    let mut c = diff(&cli_cmds, &cmds, "cmd");
    let a = diff(&cli_args, &args, "arg");
    c.unclassified.extend(a.unclassified);
    c.stale.extend(a.stale);

    // A `gui` claim must name a view that exists.
    let views = repo_root().join("ui/src/views");
    for (key, entry) in cmds.iter().chain(args.iter()) {
        if let Ok(gui) = entry {
            let view = gui
                .strip_prefix("TODO:")
                .unwrap_or(gui)
                .split('/')
                .next()
                .unwrap_or("");
            if !views.join(format!("{view}.vue")).is_file() {
                problems.push(format!("\"{key}\": no ui/src/views/{view}.vue"));
            }
        }
    }

    let todo = cmds
        .values()
        .chain(args.values())
        .filter(|e| matches!(e, Ok(g) if g.starts_with("TODO:")))
        .count();
    let surfaced = cmds.len() + args.len()
        - todo
        - cmds
            .values()
            .chain(args.values())
            .filter(|e| e.is_err())
            .count();
    println!(
        "GUI parity: {} commands, {} args; {surfaced} surfaced, {todo} TODO, {} excluded",
        cli_cmds.len(),
        cli_args.len(),
        cmds.values()
            .chain(args.values())
            .filter(|e| e.is_err())
            .count()
    );

    assert!(
        c.unclassified.is_empty() && c.stale.is_empty() && problems.is_empty(),
        "ui/parity.toml is out of step with the CLI.\n\
         Unclassified (add to ui/parity.toml):\n  {}\n\
         Stale (remove from ui/parity.toml):\n  {}\n\
         Invalid:\n  {}",
        c.unclassified.join("\n  "),
        c.stale.join("\n  "),
        problems.join("\n  ")
    );
}

// ---- the gate itself must be able to fail ----

#[test]
fn surface_keys_flags_short_only_positionals_and_skips_globals() {
    let t = serde_json::json!({"name":"mediagit","args":[
        {"id":"machine","long":"machine"},{"id":"help","long":"help"}],
      "subcommands":[{"name":"push","args":[
        {"id":"machine","long":"machine"},
        {"id":"all","long":"all","short":"a"},
        {"id":"n","short":"n"},
        {"id":"remote_path","value_name":"REMOTE_PATH","positional":true},
        {"id":"help","long":"help"}],
        "subcommands":[{"name":"help","args":[],"subcommands":[]}]}]});
    let (cmds, args) = surface(&t);
    assert_eq!(cmds, BTreeSet::from(["push".to_string()]));
    let want = ["--machine", "push --all", "push -n", "push REMOTE_PATH"];
    assert_eq!(args, want.iter().map(|s| s.to_string()).collect());
}

#[test]
fn diff_reports_unclassified_and_stale_entries() {
    let cli: BTreeSet<String> = ["push --force", "push --tags"].map(String::from).into();
    let mut classified = BTreeMap::new();
    classified.insert("push --tags".to_string(), Ok("SyncView".to_string()));
    classified.insert("push --gone".to_string(), Err("why".to_string()));
    let d = diff(&cli, &classified, "arg");
    assert_eq!(d.unclassified, ["arg push --force"]);
    assert_eq!(d.stale, ["arg push --gone"]);
}

#[test]
fn entry_with_neither_or_both_keys_is_invalid() {
    let doc: toml::Table =
        "[arg.\"a\"]\n[arg.\"b\"]\ngui=\"X\"\nexcluded=\"y\"\n[arg.\"c\"]\ngui=\"X\"\n"
            .parse()
            .unwrap();
    let mut problems = Vec::new();
    let t = load_table(&doc, "arg", &mut problems);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert_eq!(t.keys().collect::<Vec<_>>(), ["c"]);
}
