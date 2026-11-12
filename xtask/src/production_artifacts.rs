// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Authenticates digest-selected owner inputs and publishes their canonical frames.
//!
//! The command checks finite limits, distinct paths, and the published command
//! and request decoders. It writes those canonical frames only after every
//! check succeeds. It grants no authority, opens no native provider, and
//! starts no process.

use rustix::fs::{CWD, OFlags, RenameFlags, renameat_with};
use sha2::{Digest, Sha256};
use sim_codec_binary::{
    BinaryFrameLane, DecodeLimits, decode_canonical_located_tree_frame_with_limits, encode_frame,
};
use sim_kernel::{CodecId, Datum, Expr};
use sim_lib_exec::{CommandSpec, LocalCheckRequest};
use sim_platform_linux::CgroupV2Limits;
use sim_platform_ubuntu_pc::ReadOnlyTreeLimits;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

const MATERIALIZATION_BYTES: usize = 2 * 1024 * 1024;
const UNIT_GRAPH_BYTES: usize = 32 * 1024 * 1024;
const ANALYZER_BYTES: usize = 128 * 1024 * 1024;
const DATUM_BYTES: usize = 256 * 1024;
const OUTPUT_NAMES: [&str; 3] = ["build-job.sxb", "continuation-job.sxb", "startup.sxb"];

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    if matches!(args.get(2).map(String::as_str), Some("help" | "--help")) {
        print!("{USAGE}");
        return Ok(());
    }
    let input = Inputs::parse(args.into_iter().skip(2))?;
    let prepared = prepare(&input)
        .map_err(|error| format!("production artifact composition failed: {error}"))?;
    publish(&input.output_directory, &prepared)
        .map_err(|error| format!("production artifact publication failed: {error}"))?;
    for (name, bytes) in [
        (OUTPUT_NAMES[0], prepared.build_job.as_slice()),
        (OUTPUT_NAMES[1], prepared.continuation_job.as_slice()),
        (OUTPUT_NAMES[2], prepared.startup.as_slice()),
    ] {
        println!(
            "{}\tsha256:{}",
            input.output_directory.join(name).display(),
            encode_digest(Sha256::digest(bytes).into())
        );
    }
    Ok(())
}

struct Inputs {
    materialization: OwnerFile,
    build_command: OwnerFile,
    build_request: OwnerFile,
    continuation_command: OwnerFile,
    continuation_request: OwnerFile,
    installation: OwnerFile,
    volume: OwnerFile,
    boundary: OwnerFile,
    controller: OwnerFile,
    unit_graph: OwnerFile,
    analyzer: OwnerFile,
    output_directory: PathBuf,
    build: BuildInputs,
    continuation: ContinuationInputs,
    startup: StartupInputs,
    roadmap: String,
    phase: String,
    target_triple: String,
    source_path_mappings: BTreeMap<String, String>,
}

struct BuildInputs {
    bwrap: PathBuf,
    prlimit: PathBuf,
    cargo: PathBuf,
    source_root: PathBuf,
    vendor_root: PathBuf,
    sources: BTreeMap<String, PathBuf>,
    source_limits: BTreeMap<String, ReadOnlyTreeLimits>,
    operation_journal: PathBuf,
    roadmap_journal: PathBuf,
    uid: u32,
    lease_ms: u64,
}

struct ContinuationInputs {
    bwrap: PathBuf,
    prlimit: PathBuf,
    retained_artifact: PathBuf,
    sources: BTreeMap<String, PathBuf>,
    operation_journal: PathBuf,
    uid: u32,
    lease_ms: u64,
}

struct StartupInputs {
    journal: PathBuf,
    parent: String,
    supervisor: String,
    query_name: String,
    parent_limits: CgroupV2Limits,
    query_limits: CgroupV2Limits,
    timeout_ms: u64,
}

#[derive(Clone)]
struct OwnerFile {
    path: PathBuf,
    sha256: [u8; 32],
}

struct PreparedArtifacts {
    build_job: Vec<u8>,
    continuation_job: Vec<u8>,
    startup: Vec<u8>,
}

impl Inputs {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut flags = Flags::parse(args)?;
        let output_directory = flags.absolute_path("--output-dir")?;
        let materialization = flags.owner_file("--materialization")?;
        let build_command = flags.owner_file("--build-command")?;
        let build_request = flags.owner_file("--build-request")?;
        let continuation_command = flags.owner_file("--continuation-command")?;
        let continuation_request = flags.owner_file("--continuation-request")?;
        let installation = flags.owner_file("--installation")?;
        let volume = flags.owner_file("--volume")?;
        let boundary = flags.owner_file("--boundary")?;
        let controller = flags.owner_file("--controller")?;
        let unit_graph = flags.owner_file("--unit-graph")?;
        let analyzer = flags.owner_file("--analyzer")?;

        let build = BuildInputs {
            bwrap: flags.absolute_path("--build-bwrap")?,
            prlimit: flags.absolute_path("--build-prlimit")?,
            cargo: flags.absolute_path("--build-cargo")?,
            source_root: flags.absolute_path("--source-root")?,
            vendor_root: flags.absolute_path("--vendor-root")?,
            sources: flags.path_map("--build-source", 14)?,
            source_limits: flags.limit_map("--build-limit", 14)?,
            operation_journal: flags.absolute_path("--build-operation-journal")?,
            roadmap_journal: flags.absolute_path("--build-roadmap-journal")?,
            uid: flags.number("--build-uid")?,
            lease_ms: flags.number("--build-lease-ms")?,
        };
        let continuation = ContinuationInputs {
            bwrap: flags.absolute_path("--continuation-bwrap")?,
            prlimit: flags.absolute_path("--continuation-prlimit")?,
            retained_artifact: flags.absolute_path("--retained-artifact")?,
            sources: flags.path_map("--continuation-source", 16)?,
            operation_journal: flags.absolute_path("--continuation-operation-journal")?,
            uid: flags.number("--continuation-uid")?,
            lease_ms: flags.number("--continuation-lease-ms")?,
        };
        let startup = StartupInputs {
            journal: flags.absolute_path("--startup-journal")?,
            parent: flags.one("--startup-parent")?,
            supervisor: flags.one("--startup-supervisor")?,
            query_name: flags.one("--startup-query-name")?,
            parent_limits: flags.cgroup_limits("parent")?,
            query_limits: flags.cgroup_limits("query")?,
            timeout_ms: flags.number("--startup-timeout-ms")?,
        };
        let roadmap = flags.one("--roadmap")?;
        let phase = flags.one("--phase")?;
        let target_triple = flags.one("--target-triple")?;
        let source_path_mappings = flags.string_map("--source-map", 256)?;
        flags.finish()?;

        Ok(Self {
            materialization,
            build_command,
            build_request,
            continuation_command,
            continuation_request,
            installation,
            volume,
            boundary,
            controller,
            unit_graph,
            analyzer,
            output_directory,
            build,
            continuation,
            startup,
            roadmap,
            phase,
            target_triple,
            source_path_mappings,
        })
    }
}

struct Flags(BTreeMap<String, Vec<String>>);

impl Flags {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        if args.next().as_deref() != Some("emit") {
            return Err("usage: production-artifacts emit --output-dir ABSOLUTE ...".into());
        }
        let mut values = BTreeMap::<String, Vec<String>>::new();
        while let Some(flag) = args.next() {
            if !flag.starts_with("--") || !KNOWN_FLAGS.contains(&flag.as_str()) {
                return Err(format!("unknown production-artifacts argument `{flag}`"));
            }
            let value = args
                .next()
                .ok_or_else(|| format!("{flag} requires one value"))?;
            if value.starts_with("--") || value.is_empty() || value.len() > 4096 {
                return Err(format!("{flag} has an invalid or unbounded value"));
            }
            values.entry(flag).or_default().push(value);
        }
        Ok(Self(values))
    }

    fn one(&mut self, flag: &str) -> Result<String, String> {
        let values = self
            .0
            .remove(flag)
            .ok_or_else(|| format!("missing required {flag}"))?;
        let [value] = values.as_slice() else {
            return Err(format!("{flag} must occur exactly once"));
        };
        Ok(value.clone())
    }

    fn owner_file(&mut self, flag: &str) -> Result<OwnerFile, String> {
        let path = self.absolute_path(flag)?;
        let digest_flag = format!("{flag}-sha256");
        let sha256 = parse_digest(&self.one(&digest_flag)?)?;
        Ok(OwnerFile { path, sha256 })
    }

    fn absolute_path(&mut self, flag: &str) -> Result<PathBuf, String> {
        let value = self.one(flag)?;
        let path = PathBuf::from(&value);
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(format!("{flag} must be an absolute path without `..`"));
        }
        Ok(path)
    }

    fn number<T>(&mut self, flag: &str) -> Result<T, String>
    where
        T: std::str::FromStr + ToString,
    {
        let text = self.one(flag)?;
        let value = text
            .parse::<T>()
            .map_err(|_| format!("{flag} is not an unsigned decimal integer"))?;
        if value.to_string() != text {
            return Err(format!(
                "{flag} is not a canonical unsigned decimal integer"
            ));
        }
        Ok(value)
    }

    fn path_map(
        &mut self,
        flag: &str,
        maximum: usize,
    ) -> Result<BTreeMap<String, PathBuf>, String> {
        self.string_map(flag, maximum)?
            .into_iter()
            .map(|(name, value)| {
                let path = PathBuf::from(&value);
                if !path.is_absolute()
                    || path
                        .components()
                        .any(|part| matches!(part, std::path::Component::ParentDir))
                {
                    return Err(format!("{flag} value for `{name}` is not an absolute path"));
                }
                Ok((name, path))
            })
            .collect()
    }

    fn string_map(
        &mut self,
        flag: &str,
        maximum: usize,
    ) -> Result<BTreeMap<String, String>, String> {
        let values = self.0.remove(flag).unwrap_or_default();
        if values.len() > maximum {
            return Err(format!("{flag} exceeds its finite count"));
        }
        let mut result = BTreeMap::new();
        for value in values {
            let (key, item) = value
                .split_once('=')
                .ok_or_else(|| format!("{flag} must be NAME=VALUE"))?;
            if key.is_empty() || item.is_empty() || result.insert(key.into(), item.into()).is_some()
            {
                return Err(format!("{flag} contains an empty or duplicate key"));
            }
        }
        Ok(result)
    }

    fn limit_map(
        &mut self,
        flag: &str,
        maximum: usize,
    ) -> Result<BTreeMap<String, ReadOnlyTreeLimits>, String> {
        let raw = self.string_map(flag, maximum)?;
        raw.into_iter()
            .map(|(name, value)| {
                let (entries, bytes) = value
                    .split_once(':')
                    .ok_or_else(|| format!("{flag} must be ID=ENTRIES:BYTES"))?;
                let entries = canonical_number(entries, flag)?;
                let bytes = canonical_number(bytes, flag)?;
                let limits = ReadOnlyTreeLimits::production_input(entries, bytes)
                    .map_err(|error| format!("{flag} for `{name}` is invalid: {error}"))?;
                Ok((name, limits))
            })
            .collect()
    }

    fn cgroup_limits(&mut self, prefix: &str) -> Result<CgroupV2Limits, String> {
        let name = |suffix: &str| format!("--startup-{prefix}-{suffix}");
        Ok(CgroupV2Limits {
            cpu_quota_us: self.number(&name("cpu-quota-us"))?,
            cpu_period_us: self.number(&name("cpu-period-us"))?,
            cpu_burst_us: self.number(&name("cpu-burst-us"))?,
            memory_bytes: self.number(&name("memory-bytes"))?,
            swap_bytes: self.number(&name("swap-bytes"))?,
            tasks: self.number(&name("tasks"))?,
            group_oom_kill: match self.one(&name("group-oom-kill"))?.as_str() {
                "0" => false,
                "1" => true,
                _ => return Err(format!("{} must be 0 or 1", name("group-oom-kill"))),
            },
        })
    }

    fn finish(self) -> Result<(), String> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "unused production-artifacts arguments: {:?}",
                self.0.keys()
            ))
        }
    }
}

fn prepare(input: &Inputs) -> io::Result<PreparedArtifacts> {
    if input.output_directory.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "production artifact output directory already exists",
        ));
    }
    let materialization = read_owner_file(&input.materialization, MATERIALIZATION_BYTES)?;
    let unit_graph = read_owner_file(&input.unit_graph, UNIT_GRAPH_BYTES)?;
    let analyzer = read_owner_file(&input.analyzer, ANALYZER_BYTES)?;
    let installation = read_owner_file(&input.installation, DATUM_BYTES)?;
    let boundary = read_owner_file(&input.boundary, DATUM_BYTES)?;
    let controller = read_owner_file(&input.controller, DATUM_BYTES)?;
    let build_command = decode_command(&input.build_command)?;
    let build_request = decode_request(&input.build_request)?;
    let continuation_command = decode_command(&input.continuation_command)?;
    let continuation_request = decode_request(&input.continuation_request)?;
    let volume = decode_datum(&input.volume)?;
    admit(
        input,
        &AdmittedBytes {
            materialization: &materialization,
            unit_graph: &unit_graph,
            analyzer: &analyzer,
            installation: &installation,
            boundary: &boundary,
            controller: &controller,
            build_command: &build_command,
            build_request: &build_request,
            continuation_command: &continuation_command,
            continuation_request: &continuation_request,
        },
    )?;
    Ok(PreparedArtifacts {
        build_job: encode_datum(&build_command.canonical_datum())?,
        continuation_job: encode_datum(&continuation_command.canonical_datum())?,
        startup: encode_datum(&volume)?,
    })
}

struct AdmittedBytes<'a> {
    materialization: &'a [u8],
    unit_graph: &'a [u8],
    analyzer: &'a [u8],
    installation: &'a [u8],
    boundary: &'a [u8],
    controller: &'a [u8],
    build_command: &'a CommandSpec,
    build_request: &'a LocalCheckRequest,
    continuation_command: &'a CommandSpec,
    continuation_request: &'a LocalCheckRequest,
}

fn admit(input: &Inputs, bytes: &AdmittedBytes<'_>) -> io::Result<()> {
    confirm_digest(&input.materialization, bytes.materialization)?;
    confirm_digest(&input.unit_graph, bytes.unit_graph)?;
    confirm_digest(&input.analyzer, bytes.analyzer)?;
    confirm_digest(&input.installation, bytes.installation)?;
    confirm_digest(&input.boundary, bytes.boundary)?;
    confirm_digest(&input.controller, bytes.controller)?;
    distinct_owner_paths(input)?;
    admit_job_paths(input)?;
    bounded_label(&input.roadmap)?;
    bounded_label(&input.phase)?;
    bounded_label(&input.startup.parent)?;
    bounded_label(&input.startup.supervisor)?;
    validate_target(&input.target_triple)?;
    validate_source_mappings(&input.source_path_mappings)?;
    finite_identity(input.build.uid, input.build.lease_ms)?;
    finite_identity(input.continuation.uid, input.continuation.lease_ms)?;
    if input.startup.timeout_ms == 0 {
        return Err(invalid("startup timeout is not finite"));
    }
    CgroupV2Limits::admit_child(
        &input.startup.query_name,
        input.startup.query_limits,
        input.startup.parent_limits,
    )?;
    same_command(bytes.build_command, bytes.build_request)?;
    same_command(bytes.continuation_command, bytes.continuation_request)?;
    refuse_overlap(
        &input.build.sources,
        &input.build.operation_journal,
        &input.build.roadmap_journal,
    )?;
    refuse_overlap(
        &input.continuation.sources,
        &input.continuation.operation_journal,
        &input.continuation.retained_artifact,
    )?;
    for limits in input.build.source_limits.values() {
        if limits.maximum_entries() == 0 || limits.maximum_bytes() == 0 {
            return Err(invalid("read-only tree limit is empty"));
        }
    }
    Ok(())
}

fn confirm_digest(file: &OwnerFile, bytes: &[u8]) -> io::Result<()> {
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    if actual == file.sha256 {
        Ok(())
    } else {
        Err(invalid("owner input digest changed after it was read"))
    }
}

fn admit_job_paths(input: &Inputs) -> io::Result<()> {
    distinct_paths(&[
        input.build.bwrap.as_path(),
        input.build.prlimit.as_path(),
        input.build.cargo.as_path(),
    ])?;
    distinct_paths(&[
        input.continuation.bwrap.as_path(),
        input.continuation.prlimit.as_path(),
    ])?;
    let data = [
        input.build.source_root.as_path(),
        input.build.vendor_root.as_path(),
        input.build.operation_journal.as_path(),
        input.build.roadmap_journal.as_path(),
        input.continuation.retained_artifact.as_path(),
        input.continuation.operation_journal.as_path(),
        input.startup.journal.as_path(),
    ];
    distinct_paths(&data)?;
    for tool in [
        input.build.bwrap.as_path(),
        input.build.prlimit.as_path(),
        input.build.cargo.as_path(),
        input.continuation.bwrap.as_path(),
        input.continuation.prlimit.as_path(),
    ] {
        if data.contains(&tool) {
            return Err(invalid("job tools do not name distinct paths"));
        }
    }
    Ok(())
}

fn distinct_paths(paths: &[&Path]) -> io::Result<()> {
    for (index, path) in paths.iter().enumerate() {
        if paths.iter().skip(index + 1).any(|other| *other == *path) {
            return Err(invalid("job tools do not name distinct paths"));
        }
    }
    Ok(())
}

fn distinct_owner_paths(input: &Inputs) -> io::Result<()> {
    let paths = [
        &input.materialization.path,
        &input.build_command.path,
        &input.build_request.path,
        &input.continuation_command.path,
        &input.continuation_request.path,
        &input.installation.path,
        &input.volume.path,
        &input.boundary.path,
        &input.controller.path,
        &input.unit_graph.path,
        &input.analyzer.path,
    ];
    for (index, path) in paths.iter().enumerate() {
        if paths.iter().skip(index + 1).any(|other| *other == *path) {
            return Err(invalid("owner inputs do not name distinct paths"));
        }
    }
    Ok(())
}

fn bounded_label(value: &str) -> io::Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-/".contains(&byte))
    {
        return Err(invalid("production label is invalid"));
    }
    Ok(())
}

fn validate_target(target: &str) -> io::Result<()> {
    if target.is_empty()
        || target.len() > 256
        || !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(invalid("target triple is invalid"));
    }
    Ok(())
}

fn validate_source_mappings(mappings: &BTreeMap<String, String>) -> io::Result<()> {
    if mappings.is_empty() || mappings.len() > 256 {
        return Err(invalid("source mapping count is not finite"));
    }
    let paths = mappings
        .iter()
        .map(|(host, guest)| {
            let host = Path::new(host);
            let guest = Path::new(guest);
            if !host.is_absolute() || !guest.is_absolute() || has_parent(host) || has_parent(guest)
            {
                return Err(invalid("source mapping is not an absolute path"));
            }
            Ok((host, guest))
        })
        .collect::<io::Result<Vec<_>>>()?;
    for (index, (host, guest)) in paths.iter().enumerate() {
        if paths
            .iter()
            .skip(index + 1)
            .any(|(other_host, other_guest)| {
                host.starts_with(other_host)
                    || other_host.starts_with(host)
                    || guest.starts_with(other_guest)
                    || other_guest.starts_with(guest)
            })
        {
            return Err(invalid("source mappings overlap"));
        }
    }
    Ok(())
}

fn has_parent(path: &Path) -> bool {
    path.components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
}

fn finite_identity(uid: u32, lease_ms: u64) -> io::Result<()> {
    if uid == 0 || uid == u32::MAX || !(1..=90_000).contains(&lease_ms) {
        return Err(invalid("job identity or lease is not finite"));
    }
    Ok(())
}

fn same_command(command: &CommandSpec, request: &LocalCheckRequest) -> io::Result<()> {
    if request.command() == command.id() {
        Ok(())
    } else {
        Err(invalid("local check request names a different command"))
    }
}

fn refuse_overlap(
    sources: &BTreeMap<String, PathBuf>,
    journal: &Path,
    other: &Path,
) -> io::Result<()> {
    if sources
        .values()
        .any(|source| source == journal || source == other)
        || journal == other
    {
        return Err(invalid("a job path repeats a source or journal"));
    }
    Ok(())
}

fn encode_datum(value: &Datum) -> io::Result<Vec<u8>> {
    let frame = encode_frame(&Expr::from(value.clone())).map_err(io::Error::other)?;
    Ok(frame.0)
}

fn decode_command(file: &OwnerFile) -> io::Result<CommandSpec> {
    CommandSpec::from_datum(&decode_datum(file)?).map_err(io::Error::other)
}

fn decode_request(file: &OwnerFile) -> io::Result<LocalCheckRequest> {
    LocalCheckRequest::from_datum(&decode_datum(file)?).map_err(io::Error::other)
}

fn decode_datum(file: &OwnerFile) -> io::Result<Datum> {
    let bytes = read_owner_file(file, DATUM_BYTES)?;
    let limits = DecodeLimits {
        max_frame_bytes: DATUM_BYTES,
        max_expr_nodes: 4096,
        max_depth: 24,
        max_string_bytes: 4096,
        max_blob_bytes: 16 * 1024,
        max_table_entries: 256,
        max_trivia_items: 0,
    };
    let (_, tree) = decode_canonical_located_tree_frame_with_limits(
        CodecId(0),
        &bytes,
        BinaryFrameLane::Bare,
        limits,
    )
    .map_err(io::Error::other)?;
    Datum::try_from(tree.expr).map_err(io::Error::other)
}

fn read_owner_file(file: &OwnerFile, maximum: usize) -> io::Result<Vec<u8>> {
    if !file.path.is_absolute() {
        return Err(invalid("owner input path is not absolute"));
    }
    // Open the final path component itself. A later replacement of that
    // component cannot retarget this descriptor, and a fifo cannot block it.
    let descriptor = rustix::fs::open(
        &file.path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|err| {
        if err == rustix::io::Errno::LOOP {
            invalid("owner input is not a regular non-symlink file")
        } else {
            io::Error::from(err)
        }
    })?;
    let stat = rustix::fs::fstat(&descriptor)?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(invalid("owner input is not a regular non-symlink file"));
    }
    let opened_length = u64::try_from(stat.st_size)
        .map_err(|_| invalid("owner input length exceeds this platform"))?;
    if opened_length == 0 || opened_length > maximum as u64 {
        return Err(invalid("owner input exceeds its finite byte bound"));
    }
    let mut handle = File::from(descriptor);
    let length = usize::try_from(opened_length)
        .map_err(|_| invalid("owner input length exceeds this platform"))?;
    let mut bytes = Vec::with_capacity(length);
    Read::by_ref(&mut handle)
        .take((maximum as u64) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() != length || bytes.len() > maximum {
        return Err(invalid(
            "owner input changed or exceeded its finite byte bound",
        ));
    }
    let actual: [u8; 32] = Sha256::digest(&bytes).into();
    if actual != file.sha256 {
        return Err(invalid("owner input digest differs"));
    }
    Ok(bytes)
}

fn publish(output: &Path, artifacts: &PreparedArtifacts) -> io::Result<()> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| invalid("output directory has no parent"))?;
    let leaf = output
        .file_name()
        .ok_or_else(|| invalid("output directory has no leaf name"))?;
    if !output.is_absolute() || output.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "production artifact output must be a fresh absolute directory",
        ));
    }
    let stage = create_stage(parent, leaf)?;
    let result = (|| {
        for (name, bytes) in [
            (OUTPUT_NAMES[0], artifacts.build_job.as_slice()),
            (OUTPUT_NAMES[1], artifacts.continuation_job.as_slice()),
            (OUTPUT_NAMES[2], artifacts.startup.as_slice()),
        ] {
            let path = stage.join(name);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        File::open(&stage)?.sync_all()?;
        renameat_with(CWD, &stage, CWD, output, RenameFlags::NOREPLACE)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() && stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

fn create_stage(parent: &Path, output_leaf: &std::ffi::OsStr) -> io::Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let leaf = output_leaf
        .to_str()
        .ok_or_else(|| invalid("output directory leaf is not UTF-8"))?;
    for attempt in 0..16_u8 {
        let candidate = parent.join(format!(".{leaf}.stage-{}-{nonce}-{attempt}", process::id()));
        match fs::DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a fresh artifact staging directory",
    ))
}

fn parse_digest(text: &str) -> Result<[u8; 32], String> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("SHA-256 digest must be 64 lowercase hexadecimal characters".into());
    }
    let mut digest = [0_u8; 32];
    for (index, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let pair = std::str::from_utf8(pair).map_err(|_| "SHA-256 digest is not ASCII")?;
        digest[index] = u8::from_str_radix(pair, 16).map_err(|_| "SHA-256 digest is invalid")?;
    }
    Ok(digest)
}

fn encode_digest(digest: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn canonical_number(text: &str, flag: &str) -> Result<usize, String> {
    let number = text
        .parse::<usize>()
        .map_err(|_| format!("{flag} contains an invalid unsigned integer"))?;
    if number.to_string() != text {
        return Err(format!("{flag} contains a noncanonical unsigned integer"));
    }
    Ok(number)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

const KNOWN_FLAGS: &[&str] = &[
    "--output-dir",
    "--materialization",
    "--materialization-sha256",
    "--build-command",
    "--build-command-sha256",
    "--build-request",
    "--build-request-sha256",
    "--continuation-command",
    "--continuation-command-sha256",
    "--continuation-request",
    "--continuation-request-sha256",
    "--installation",
    "--installation-sha256",
    "--volume",
    "--volume-sha256",
    "--boundary",
    "--boundary-sha256",
    "--controller",
    "--controller-sha256",
    "--unit-graph",
    "--unit-graph-sha256",
    "--analyzer",
    "--analyzer-sha256",
    "--build-bwrap",
    "--build-prlimit",
    "--build-cargo",
    "--source-root",
    "--vendor-root",
    "--build-source",
    "--build-limit",
    "--build-operation-journal",
    "--build-roadmap-journal",
    "--build-uid",
    "--build-lease-ms",
    "--continuation-bwrap",
    "--continuation-prlimit",
    "--retained-artifact",
    "--continuation-source",
    "--continuation-operation-journal",
    "--continuation-uid",
    "--continuation-lease-ms",
    "--startup-journal",
    "--startup-parent",
    "--startup-supervisor",
    "--startup-query-name",
    "--startup-parent-cpu-quota-us",
    "--startup-parent-cpu-period-us",
    "--startup-parent-cpu-burst-us",
    "--startup-parent-memory-bytes",
    "--startup-parent-swap-bytes",
    "--startup-parent-tasks",
    "--startup-parent-group-oom-kill",
    "--startup-query-cpu-quota-us",
    "--startup-query-cpu-period-us",
    "--startup-query-cpu-burst-us",
    "--startup-query-memory-bytes",
    "--startup-query-swap-bytes",
    "--startup-query-tasks",
    "--startup-query-group-oom-kill",
    "--startup-timeout-ms",
    "--roadmap",
    "--phase",
    "--target-triple",
    "--source-map",
];

const USAGE: &str = "\
usage: cargo run -p xtask -- production-artifacts emit \\
  --output-dir ABSOLUTE \\
  --materialization PATH --materialization-sha256 HEX \\
  --build-command PATH --build-command-sha256 HEX \\
  --build-request PATH --build-request-sha256 HEX \\
  --continuation-command PATH --continuation-command-sha256 HEX \\
  --continuation-request PATH --continuation-request-sha256 HEX \\
  --installation PATH --installation-sha256 HEX \\
  --volume PATH --volume-sha256 HEX \\
  --boundary PATH --boundary-sha256 HEX \\
  --controller PATH --controller-sha256 HEX \\
  --unit-graph PATH --unit-graph-sha256 HEX \\
  --analyzer PATH --analyzer-sha256 HEX \\
  --build-bwrap ABSOLUTE --build-prlimit ABSOLUTE --build-cargo ABSOLUTE \\
  --source-root ABSOLUTE --vendor-root ABSOLUTE \\
  [--build-source ID=ABSOLUTE]... [--build-limit ID=ENTRIES:BYTES]... \\
  --build-operation-journal ABSOLUTE --build-roadmap-journal ABSOLUTE \\
  --build-uid NUMBER --build-lease-ms NUMBER \\
  --continuation-bwrap ABSOLUTE --continuation-prlimit ABSOLUTE \\
  --retained-artifact ABSOLUTE [--continuation-source ID=ABSOLUTE]... \\
  --continuation-operation-journal ABSOLUTE \\
  --continuation-uid NUMBER --continuation-lease-ms NUMBER \\
  --startup-journal ABSOLUTE --startup-parent NAME --startup-supervisor NAME \\
  --startup-query-name NAME --startup-timeout-ms NUMBER \\
  --startup-parent-cpu-quota-us NUMBER --startup-parent-cpu-period-us NUMBER \\
  --startup-parent-cpu-burst-us NUMBER --startup-parent-memory-bytes NUMBER \\
  --startup-parent-swap-bytes NUMBER --startup-parent-tasks NUMBER \\
  --startup-parent-group-oom-kill 0|1 \\
  --startup-query-cpu-quota-us NUMBER --startup-query-cpu-period-us NUMBER \\
  --startup-query-cpu-burst-us NUMBER --startup-query-memory-bytes NUMBER \\
  --startup-query-swap-bytes NUMBER --startup-query-tasks NUMBER \\
  --startup-query-group-oom-kill 0|1 \\
  --roadmap LABEL --phase LABEL --target-triple TRIPLE \\
  [--source-map HOST=GUEST]...\n";

#[cfg(test)]
#[path = "production_artifacts/tests.rs"]
mod tests;
