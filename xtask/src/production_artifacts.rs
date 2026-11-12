// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pure assembly and all-or-nothing publication of production startup artifacts.
//!
//! This command is deliberately a join at the platform owner's boundary. It
//! consumes already produced, digest-selected owner inputs and delegates every
//! wire format to the public typed decoders and composers. It grants no
//! authority, opens no native provider, and starts no process.

use rustix::fs::{CWD, RenameFlags, renameat_with};
use sha2::{Digest, Sha256};
use sim_codec_binary::{
    BinaryFrameLane, DecodeLimits, decode_canonical_located_tree_frame_with_limits,
};
use sim_kernel::{CodecId, Datum};
use sim_lib_exec::{CommandSpec, LocalCheckRequest};
use sim_platform_linux::CgroupV2Limits;
use sim_platform_ubuntu_pc::{
    ProductionProjectionStartupInput, ProductionProtectedInput, ProjectionWorkerJobInput,
    ReadOnlyTreeLimits, UbuntuAdministrativeStartup,
};
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

struct StartupOwnerInputs<'a> {
    materialization: &'a [u8],
    installation: &'a [u8],
    volume: &'a Datum,
    boundary: &'a [u8],
    controller: &'a [u8],
    build_job: &'a [u8],
    continuation_job: &'a [u8],
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
    let build_command = decode_command(&input.build_command)?;
    let build_request = decode_request(&input.build_request)?;
    let continuation_command = decode_command(&input.continuation_command)?;
    let continuation_request = decode_request(&input.continuation_request)?;
    let installation = read_owner_file(&input.installation, DATUM_BYTES)?;
    let volume = decode_datum(&input.volume)?;
    let boundary = read_owner_file(&input.boundary, DATUM_BYTES)?;
    let controller = read_owner_file(&input.controller, DATUM_BYTES)?;

    let build_job = compose_build_job(
        input,
        &materialization,
        &unit_graph,
        &analyzer,
        &build_command,
        &build_request,
    )?;
    let continuation_job =
        compose_continuation_job(input, &continuation_command, &continuation_request)?;
    let startup = compose_startup(
        input,
        &StartupOwnerInputs {
            materialization: &materialization,
            installation: &installation,
            volume: &volume,
            boundary: &boundary,
            controller: &controller,
            build_job: &build_job,
            continuation_job: &continuation_job,
        },
    )?;
    Ok(PreparedArtifacts {
        build_job,
        continuation_job,
        startup,
    })
}

fn compose_build_job(
    input: &Inputs,
    materialization: &[u8],
    unit_graph: &[u8],
    analyzer: &[u8],
    build_command: &CommandSpec,
    build_request: &LocalCheckRequest,
) -> io::Result<Vec<u8>> {
    let profile = UbuntuAdministrativeStartup::compose_projection_build_profile(
        materialization,
        input.materialization.sha256,
        &input.roadmap,
        &input.phase,
    )?;
    let (source_id, vendor_id) =
        UbuntuAdministrativeStartup::production_materialization_resource_ids(
            materialization,
            input.materialization.sha256,
        )?;
    let mut build_sources = input.build.sources.clone();
    for (identity, path) in [
        (source_id.clone(), input.build.source_root.clone()),
        (vendor_id.clone(), input.build.vendor_root.clone()),
    ] {
        if build_sources.insert(identity, path).is_some() {
            return Err(invalid(
                "a materialization resource was also supplied as --build-source",
            ));
        }
    }
    let mut source_limits = input.build.source_limits.clone();
    for (identity, limit) in
        UbuntuAdministrativeStartup::production_materialization_resource_limits(
            materialization,
            input.materialization.sha256,
        )?
    {
        if source_limits.insert(identity, limit).is_some() {
            return Err(invalid(
                "a materialization resource was also supplied as --build-limit",
            ));
        }
    }

    let v2 = UbuntuAdministrativeStartup::compose_projection_build_job_v2(
        build_command,
        build_request,
        &input.build.bwrap,
        &input.build.prlimit,
        &input.build.cargo,
        &build_sources,
        &input.build.operation_journal,
        input.build.uid,
        input.build.lease_ms,
        &profile,
        &input.build.roadmap_journal,
    )?;
    UbuntuAdministrativeStartup::upgrade_projection_build_job(
        &v2,
        &input.unit_graph.path,
        unit_graph,
        &input.analyzer.path,
        analyzer,
        &input.target_triple,
        &input.source_path_mappings,
        &source_limits,
    )
}

fn compose_continuation_job(
    input: &Inputs,
    command: &CommandSpec,
    request: &LocalCheckRequest,
) -> io::Result<Vec<u8>> {
    UbuntuAdministrativeStartup::compose_projection_worker_job_v1(ProjectionWorkerJobInput {
        command,
        request,
        bwrap: &input.continuation.bwrap,
        prlimit: &input.continuation.prlimit,
        retained_artifact_path: &input.continuation.retained_artifact,
        sources: &input.continuation.sources,
        operation_journal: &input.continuation.operation_journal,
        uid: input.continuation.uid,
        lease_ms: input.continuation.lease_ms,
    })
}

fn compose_startup(input: &Inputs, owner: &StartupOwnerInputs<'_>) -> io::Result<Vec<u8>> {
    let build_path = input.output_directory.join(OUTPUT_NAMES[0]);
    let continuation_path = input.output_directory.join(OUTPUT_NAMES[1]);
    UbuntuAdministrativeStartup::compose_production_projection_startup_v9(
        ProductionProjectionStartupInput {
            installation: ProductionProtectedInput::new(
                &input.installation.path,
                owner.installation,
            )?,
            journal: &input.startup.journal,
            parent: &input.startup.parent,
            supervisor: &input.startup.supervisor,
            query_name: &input.startup.query_name,
            parent_limits: input.startup.parent_limits,
            query_limits: input.startup.query_limits,
            timeout_ms: input.startup.timeout_ms,
            volume: owner.volume,
            build_job: ProductionProtectedInput::new(&build_path, owner.build_job)?,
            projection_job: ProductionProtectedInput::new(
                &continuation_path,
                owner.continuation_job,
            )?,
            projection_boundary: ProductionProtectedInput::new(
                &input.boundary.path,
                owner.boundary,
            )?,
            projection_controller: ProductionProtectedInput::new(
                &input.controller.path,
                owner.controller,
            )?,
            materialization_record: owner.materialization,
            materialization_sha256: input.materialization.sha256,
            roadmap: &input.roadmap,
            phase: &input.phase,
        },
    )
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
    let metadata = fs::symlink_metadata(&file.path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid("owner input is not a regular non-symlink file"));
    }
    if metadata.len() == 0 || metadata.len() > maximum as u64 {
        return Err(invalid("owner input exceeds its finite byte bound"));
    }
    let mut handle = File::open(&file.path)?;
    let opened = handle.metadata()?;
    if !opened.is_file() || opened.len() != metadata.len() {
        return Err(invalid("owner input changed while it was opened"));
    }
    let opened_length = usize::try_from(opened.len())
        .map_err(|_| invalid("owner input length exceeds this platform"))?;
    let mut bytes = Vec::with_capacity(opened_length);
    Read::by_ref(&mut handle)
        .take((maximum as u64) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() != opened_length || bytes.len() > maximum {
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
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
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
