// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Thin simdoc launcher: defers to the isolated simdoc encoder.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// # Errors
///
/// Returns an error when the arguments are not a simdoc invocation, the
/// isolated encoder manifest cannot be located, or that encoder exits
/// non-zero.
pub fn run(args: &[String]) -> Result<(), String> {
    let program = args.first().map_or("xtask", String::as_str);
    if args.get(1).map(String::as_str) != Some("simdoc") {
        return Err(format!("usage: {program} simdoc [--check]"));
    }

    let root = env::current_dir().map_err(|err| format!("current dir: {err}"))?;
    let manifest = locate_simdoc_manifest(&root)?;
    let mut command = encoder_command(&root, &manifest, &args[2..])?;

    let status = command
        .status()
        .map_err(|err| format!("run isolated simdoc encoder: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "isolated simdoc encoder failed with status {status}"
        ))
    }
}

fn encoder_command(root: &Path, manifest: &Path, extra: &[String]) -> Result<Command, String> {
    let root = std::path::absolute(root)
        .map_err(|err| format!("absolute repo root {}: {err}", root.display()))?;
    let manifest = std::path::absolute(manifest)
        .map_err(|err| format!("absolute simdoc manifest {}: {err}", manifest.display()))?;
    let manifest_dir = manifest.parent().ok_or_else(|| {
        format!(
            "isolated simdoc manifest {} has no directory",
            manifest.display()
        )
    })?;
    let channel = pinned_channel(manifest_dir)?;
    // A parent `cargo run` puts its own toolchain first on PATH and exports
    // RUSTC. Invoke rustup directly and drop those overrides so the encoder
    // is built with the compiler its checkout pins.
    let mut command = Command::new("rustup");
    command.current_dir(manifest_dir);
    for inherited in ["CARGO", "RUSTC", "RUSTDOC", "RUSTUP_TOOLCHAIN"] {
        command.env_remove(inherited);
    }
    command.args([
        "run",
        &channel,
        "cargo",
        "run",
        "--locked",
        "--offline",
        "--manifest-path",
    ]);
    command.arg(&manifest);
    command.args(["--quiet", "--", "simdoc", "--repo-root"]);
    command.arg(&root);
    for arg in forwarded_simdoc_args(extra)? {
        command.arg(arg);
    }
    Ok(command)
}

fn forwarded_simdoc_args(extra: &[String]) -> Result<Vec<String>, String> {
    let mut forwarded = Vec::with_capacity(extra.len());
    let mut args = extra.iter();
    while let Some(arg) = args.next() {
        if arg == "--repo-root" {
            let value = args
                .next()
                .ok_or_else(|| "simdoc --repo-root requires a path".to_owned())?;
            let absolute = std::path::absolute(value)
                .map_err(|err| format!("absolute simdoc repo root {value}: {err}"))?;
            forwarded.push("--repo-root".to_owned());
            forwarded.push(absolute.display().to_string());
        } else {
            forwarded.push(arg.clone());
        }
    }
    Ok(forwarded)
}

fn pinned_channel(manifest_dir: &Path) -> Result<String, String> {
    let toolchain = manifest_dir.join("../../rust-toolchain.toml");
    let text = fs::read_to_string(&toolchain)
        .map_err(|err| format!("read {}: {err}", toolchain.display()))?;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("channel") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let channel = rest.trim().trim_matches('"');
        if channel.is_empty() || channel.contains(char::is_whitespace) {
            break;
        }
        return Ok(channel.to_owned());
    }
    Err(format!("{} pins no channel", toolchain.display()))
}

fn locate_simdoc_manifest(repo_root: &Path) -> Result<PathBuf, String> {
    if let Ok(path) = env::var("SIMDOC_TOOLING_MANIFEST") {
        // CI and release generation set this to the tooling workspace
        // manifest. That manifest refuses the simdoc verb.
        return isolated_manifest_from_hint(Path::new(&path));
    }
    locate_sibling_manifest(repo_root)
}

fn locate_sibling_manifest(repo_root: &Path) -> Result<PathBuf, String> {
    let isolated = repo_root
        .parent()
        .unwrap_or(repo_root)
        .join("sim-tooling")
        .join("crates")
        .join("simdoc")
        .join("Cargo.toml");
    if isolated.is_file() {
        return Ok(isolated);
    }
    Err(
        "set SIMDOC_TOOLING_MANIFEST to the sim-tooling checkout or crates/simdoc/Cargo.toml"
            .to_owned(),
    )
}

fn isolated_manifest_from_hint(hint: &Path) -> Result<PathBuf, String> {
    let isolated_rel = Path::new("crates").join("simdoc").join("Cargo.toml");
    let candidates = [
        hint.to_path_buf(),
        hint.join("Cargo.toml"),
        hint.join(&isolated_rel),
        hint.parent().unwrap_or(hint).join(&isolated_rel),
    ];
    for candidate in candidates {
        if candidate.is_file() && candidate.ends_with(&isolated_rel) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "SIMDOC_TOOLING_MANIFEST={} does not locate crates/simdoc/Cargo.toml",
        hint.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        encoder_command, isolated_manifest_from_hint, locate_sibling_manifest, pinned_channel,
    };
    use std::fs;
    use std::path::{Component, Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "sim-platform-simdoc-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            ));
            let simdoc = root.join("sim-tooling/crates/simdoc");
            fs::create_dir_all(&simdoc).expect("fixture directory");
            fs::write(simdoc.join("Cargo.toml"), "[package]\nname = \"simdoc\"\n")
                .expect("fixture manifest");
            fs::write(simdoc.join("Cargo.lock"), "# fixture\n").expect("fixture lock");
            fs::write(
                root.join("sim-tooling/rust-toolchain.toml"),
                "[toolchain]\nchannel = \"1.96.0\"\n",
            )
            .expect("fixture toolchain");
            fs::create_dir_all(root.join("repo")).expect("fixture repo");
            Self { root }
        }

        fn manifest(&self) -> PathBuf {
            self.root.join("sim-tooling/crates/simdoc/Cargo.toml")
        }

        fn repo(&self) -> PathBuf {
            self.root.join("repo")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn relative_from(base: &Path, target: &Path) -> PathBuf {
        let base: Vec<_> = base.components().collect();
        let target: Vec<_> = target.components().collect();
        let shared = base
            .iter()
            .zip(target.iter())
            .take_while(|(left, right)| left == right)
            .count();
        let mut relative = PathBuf::new();
        for _ in shared..base.len() {
            relative.push("..");
        }
        for component in &target[shared..] {
            relative.push(component.as_os_str());
        }
        if relative.as_os_str().is_empty() {
            relative.push(".");
        }
        relative
    }

    #[test]
    fn tooling_hints_resolve_to_the_isolated_encoder() {
        let fixture = Fixture::new();
        let selected = locate_sibling_manifest(&fixture.repo()).expect("isolated encoder");
        assert_eq!(selected, fixture.manifest());
        assert!(selected.with_file_name("Cargo.lock").is_file());

        let tooling = fixture.root.join("sim-tooling");
        assert_eq!(
            isolated_manifest_from_hint(&tooling.join("Cargo.toml")).expect("workspace hint"),
            selected
        );
        assert_eq!(
            isolated_manifest_from_hint(&selected).expect("isolated hint"),
            selected
        );
        assert_eq!(
            isolated_manifest_from_hint(&tooling).expect("checkout hint"),
            selected
        );
    }

    #[test]
    fn missing_tooling_hint_is_an_error() {
        let missing = Fixture::new().root.join("no-such-tooling/Cargo.toml");
        let error = isolated_manifest_from_hint(&missing).expect_err("missing hint");
        assert!(error.contains("does not locate crates/simdoc/Cargo.toml"));
    }

    #[test]
    fn encoder_runs_inside_the_pinned_simdoc_checkout() {
        let fixture = Fixture::new();
        let manifest = fixture.manifest();
        let cwd = std::env::current_dir().expect("current dir");
        let relative = relative_from(&cwd, &manifest);
        assert!(
            relative
                .components()
                .any(|part| part == Component::ParentDir)
        );
        let absolute_manifest = std::path::absolute(&relative).expect("absolute manifest");
        let channel = pinned_channel(absolute_manifest.parent().expect("simdoc directory"))
            .expect("pinned channel");
        let command = encoder_command(&fixture.repo(), &relative, &["--check".to_owned()])
            .expect("encoder command");
        let repo = std::path::absolute(fixture.repo()).expect("absolute repo");
        assert!(absolute_manifest.is_absolute());
        assert_eq!(command.get_program().to_str(), Some("rustup"));
        assert_eq!(command.get_current_dir(), absolute_manifest.parent());
        let removed = command
            .get_envs()
            .filter_map(|(key, value)| value.is_none().then_some(key))
            .collect::<Vec<_>>();
        for inherited in ["CARGO", "RUSTC", "RUSTDOC", "RUSTUP_TOOLCHAIN"] {
            assert!(removed.iter().any(|key| *key == inherited));
        }
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            args.windows(4)
                .any(|window| window == ["run", channel.as_str(), "cargo", "run"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--locked", "--offline"])
        );
        assert!(args.iter().any(|arg| arg == "--check"));
        assert!(args.iter().any(|arg| Path::new(arg) == repo.as_path()));
        assert!(args.iter().any(|arg| Path::new(arg) == absolute_manifest));
    }

    #[test]
    fn relative_repo_root_is_forwarded_absolutely() {
        let fixture = Fixture::new();
        let command = encoder_command(
            &fixture.repo(),
            &fixture.manifest(),
            &[
                "--repo-root".to_owned(),
                ".".to_owned(),
                "--check".to_owned(),
            ],
        )
        .expect("encoder command");
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let roots = args
            .windows(2)
            .filter(|window| window[0] == "--repo-root")
            .map(|window| window[1].clone())
            .collect::<Vec<_>>();
        assert!(roots.len() >= 2);
        assert!(roots.iter().all(|path| Path::new(path).is_absolute()));
        let caller = std::path::absolute(".").expect("caller directory");
        assert_eq!(Path::new(roots.last().expect("forwarded root")), caller);
        let missing = encoder_command(
            &fixture.repo(),
            &fixture.manifest(),
            &["--repo-root".to_owned()],
        )
        .expect_err("missing repo root");
        assert!(missing.contains("requires a path"));
    }
}
