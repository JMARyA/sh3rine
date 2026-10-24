//! Embed the git commit so Sentry/Hegel events carry `sh3rine@<commit>[-dirty]`.
//!
//! `SH3RINE_COMMIT` (e.g. set by a Nix build, where there is no `.git`) takes
//! precedence; otherwise ask git. Unknown commit → no release is reported.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    println!("cargo::rerun-if-env-changed=SH3RINE_COMMIT");
    // Edits to tracked sources flip the dirty flag without touching .git.
    for p in ["src", "build.rs", "Cargo.toml", "Cargo.lock"] {
        println!("cargo::rerun-if-changed={p}");
    }

    let release = match std::env::var("SH3RINE_COMMIT") {
        Ok(c) if !c.is_empty() => Some(c),
        _ => git(&["rev-parse", "HEAD"]).map(|commit| {
            // Rebuild when HEAD moves or the index changes.
            if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
                println!("cargo::rerun-if-changed={dir}/HEAD");
                println!("cargo::rerun-if-changed={dir}/index");
                if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
                    println!("cargo::rerun-if-changed={dir}/{r}");
                }
            }
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .map_or(false, |s| !s.is_empty());
            if dirty { format!("{commit}-dirty") } else { commit }
        }),
    };

    if let Some(r) = release {
        println!("cargo::rustc-env=SH3RINE_RELEASE=sh3rine@{r}");
    }
}
