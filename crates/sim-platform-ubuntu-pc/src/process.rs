use sim_lib_exec::{
    BindingValue, DispatchEvidence, PrivateArtifactRef, ProcResult, ProcessAttempt,
    ProcessCancellation, ProcessPort, ProcessReceipt, ProcessRefusal, ProcessRequest, ProgramRef,
    ProjectRootRef, StopReceipt,
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::unix::process::CommandExt as _,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

/// Ubuntu process adapter. All native process mechanics are confined here.
#[derive(Clone, Debug, Default)]
pub struct UbuntuProcess {
    programs: BTreeMap<ProgramRef, std::path::PathBuf>,
    roots: BTreeMap<ProjectRootRef, std::path::PathBuf>,
    artifacts: BTreeMap<PrivateArtifactRef, std::path::PathBuf>,
}
impl UbuntuProcess {
    /// Creates a capsule from boot-trusted native resource mappings.
    #[must_use]
    pub fn new(
        programs: BTreeMap<ProgramRef, std::path::PathBuf>,
        roots: BTreeMap<ProjectRootRef, std::path::PathBuf>,
        artifacts: BTreeMap<PrivateArtifactRef, std::path::PathBuf>,
    ) -> Self {
        Self {
            programs,
            roots,
            artifacts,
        }
    }
}
impl ProcessPort for UbuntuProcess {
    fn run(&self, request: &ProcessRequest, cancellation: &ProcessCancellation) -> ProcessAttempt {
        let Some(program) = self.programs.get(&request.program) else {
            return ProcessAttempt::NotDispatched {
                refusal: ProcessRefusal::Refused("program reference is not boot-authorized".into()),
            };
        };
        let Some(root) = self.roots.get(&request.root) else {
            return ProcessAttempt::NotDispatched {
                refusal: ProcessRefusal::Refused(
                    "project-root reference is not boot-authorized".into(),
                ),
            };
        };
        let root = match root.canonicalize() {
            Ok(v) => v,
            Err(e) => {
                return ProcessAttempt::NotDispatched {
                    refusal: ProcessRefusal::Refused(format!("project root unavailable: {e}")),
                };
            }
        };
        let mut environment = BTreeMap::new();
        for (name, value) in request.environment.iter() {
            let rendered = match value {
                BindingValue::Literal(v) => v.clone(),
                BindingValue::ProjectRoot(reference) => match self.roots.get(reference) {
                    Some(v) => v.to_string_lossy().into_owned(),
                    None => {
                        return ProcessAttempt::NotDispatched {
                            refusal: ProcessRefusal::Refused(
                                "binding references wrong or unavailable resource kind".into(),
                            ),
                        };
                    }
                },
                BindingValue::PrivateArtifact(reference) => match self.artifacts.get(reference) {
                    Some(v) => v.to_string_lossy().into_owned(),
                    None => {
                        return ProcessAttempt::NotDispatched {
                            refusal: ProcessRefusal::Refused(
                                "binding references wrong or unavailable resource kind".into(),
                            ),
                        };
                    }
                },
            };
            environment.insert(name, rendered);
        }
        if request
            .private_artifacts
            .iter()
            .any(|v| !self.artifacts.contains_key(v))
        {
            return ProcessAttempt::NotDispatched {
                refusal: ProcessRefusal::Refused("private artifact is not boot-authorized".into()),
            };
        }
        let mut command = Command::new(program);
        command
            .args(request.argv.iter().map(|v| v.as_str()))
            .current_dir(root)
            .env_clear()
            .envs(environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = match command.spawn() {
            Ok(v) => v,
            Err(e) => {
                return ProcessAttempt::NotDispatched {
                    refusal: ProcessRefusal::SpawnFailed(e.to_string()),
                };
            }
        };
        run_child(&mut child, request, cancellation)
    }
}

enum Event {
    Stdout(Result<Vec<u8>, String>),
    Stderr(Result<Vec<u8>, String>),
    Stdin(Result<(), String>),
}
struct Budget {
    left: usize,
    truncated: bool,
}
pub(crate) fn run_child(
    child: &mut Child,
    request: &ProcessRequest,
    cancellation: &ProcessCancellation,
) -> ProcessAttempt {
    let started = Instant::now();
    let deadline = started.checked_add(Duration::from_millis(request.budget.timeout_ms));
    let Some(deadline) = deadline else {
        return unknown(started, "deadline", "timeout overflow");
    };
    let Some(stdout) = child.stdout.take() else {
        return unknown(started, "capture", "stdout pipe missing");
    };
    let Some(stderr) = child.stderr.take() else {
        return unknown(started, "capture", "stderr pipe missing");
    };
    let Some(stdin) = child.stdin.take() else {
        return unknown(started, "capture", "stdin pipe missing");
    };
    let budget = Arc::new(Mutex::new(Budget {
        left: request.budget.max_output_bytes,
        truncated: false,
    }));
    let (tx, rx) = mpsc::channel();
    reader(stdout, Arc::clone(&budget), tx.clone(), true);
    reader(stderr, Arc::clone(&budget), tx.clone(), false);
    writer(stdin, request.budget.stdin.clone(), tx.clone());
    drop(tx);
    let (mut status, mut out, mut err, mut input) = (None, None, None, None);
    loop {
        if status.is_none() {
            status = match child.try_wait() {
                Ok(v) => v,
                Err(e) => return unknown(started, "reap", &e.to_string()),
            };
        }
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Stdout(v) => out = Some(v),
                Event::Stderr(v) => err = Some(v),
                Event::Stdin(v) => input = Some(v),
            }
        }
        if status.is_some() && out.is_some() && err.is_some() && input.is_some() {
            let Some(status) = status else {
                unreachable!("checked above")
            };
            let Some(out) = out.take() else {
                unreachable!("checked above")
            };
            let Some(err) = err.take() else {
                unreachable!("checked above")
            };
            let Some(input) = input.take() else {
                unreachable!("checked above")
            };
            if let Err(e) = input {
                return unknown(started, "stdin", &e);
            }
            let out = match out {
                Ok(v) => v,
                Err(e) => return unknown(started, "capture", &e),
            };
            let err = match err {
                Ok(v) => v,
                Err(e) => return unknown(started, "capture", &e),
            };
            let truncated = match budget.lock() {
                Ok(v) => v.truncated,
                Err(_) => return unknown(started, "capture", "output budget poisoned"),
            };
            let pgid = child.id();
            if group_exists(pgid) {
                if let Err(detail) = terminate_tree(child) {
                    return unknown(started, "cleanup", &detail);
                }
            }
            let elapsed_mono_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            return ProcessAttempt::Completed {
                receipt: ProcessReceipt {
                    provider: "platform/site/ubuntu-pc".into(),
                    elapsed_mono_ns,
                    result: ProcResult {
                        stdout: String::from_utf8_lossy(&out).into_owned(),
                        stderr: String::from_utf8_lossy(&err).into_owned(),
                        exit_code: status.code().unwrap_or(-1),
                        truncated,
                    },
                },
            };
        }
        let cancelled = cancellation.is_cancelled();
        if cancelled || Instant::now() >= deadline {
            let cleanup = terminate_tree(child);
            let elapsed = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            return match cleanup {
                Ok(detail) => {
                    let receipt = StopReceipt {
                        provider: "platform/site/ubuntu-pc".into(),
                        elapsed_mono_ns: elapsed,
                        cleanup: detail,
                    };
                    if cancelled {
                        ProcessAttempt::StoppedAfterCancel { receipt }
                    } else {
                        ProcessAttempt::StoppedAfterTimeout { receipt }
                    }
                }
                Err(detail) => ProcessAttempt::UnknownAfterDispatch {
                    evidence: DispatchEvidence {
                        provider: "platform/site/ubuntu-pc".into(),
                        stage: "cleanup".into(),
                        detail,
                    },
                },
            };
        }
        thread::sleep(Duration::from_millis(2));
    }
}
fn reader<R: Read + Send + 'static>(
    mut stream: R,
    budget: Arc<Mutex<Budget>>,
    tx: mpsc::Sender<Event>,
    stdout: bool,
) {
    thread::spawn(move || {
        let mut result = Vec::new();
        let mut chunk = [0; 4096];
        let value = loop {
            match stream.read(&mut chunk) {
                Ok(0) => break Ok(result),
                Ok(n) => {
                    let Ok(mut b) = budget.lock() else {
                        break Err("output budget poisoned".into());
                    };
                    let keep = n.min(b.left);
                    result.extend_from_slice(&chunk[..keep]);
                    b.left -= keep;
                    b.truncated |= keep < n;
                }
                Err(e) => break Err(format!("capture: {e}")),
            }
        };
        let _ = tx.send(if stdout {
            Event::Stdout(value)
        } else {
            Event::Stderr(value)
        });
    });
}
fn writer(mut stream: std::process::ChildStdin, input: Option<Vec<u8>>, tx: mpsc::Sender<Event>) {
    thread::spawn(move || {
        let value = input.map_or(Ok(()), |v| {
            stream.write_all(&v).map_err(|e| format!("stdin: {e}"))
        });
        drop(stream);
        let _ = tx.send(Event::Stdin(value));
    });
}
fn unknown(started: Instant, stage: &str, detail: &str) -> ProcessAttempt {
    ProcessAttempt::UnknownAfterDispatch {
        evidence: DispatchEvidence {
            provider: "platform/site/ubuntu-pc".into(),
            stage: stage.into(),
            detail: format!("{}; elapsed_ns={}", detail, started.elapsed().as_nanos()),
        },
    }
}
fn terminate_tree(child: &mut Child) -> Result<String, String> {
    let pgid = child.id();
    let term = signal_group(pgid, "TERM");
    let until = Instant::now() + Duration::from_millis(100);
    while Instant::now() < until {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let kill = signal_group(pgid, "KILL");
    let wait = child.wait().map(|_| ()).map_err(|e| format!("wait: {e}"));
    let cleanup_deadline = Instant::now() + Duration::from_millis(500);
    while group_exists(pgid) && Instant::now() < cleanup_deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let leaked = group_exists(pgid);
    if leaked {
        return Err(term
            .err()
            .or_else(|| kill.err())
            .or_else(|| wait.err())
            .unwrap_or_else(|| "descendants remained after bounded cleanup".into()));
    }
    wait.map(|()| "process group killed and reaped".into())
}
fn signal_group(pgid: u32, signal: &str) -> Result<(), String> {
    let status = Command::new("kill")
        .args([format!("-{signal}"), "--".into(), format!("-{pgid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("kill {signal} group {pgid}: {status}"))
    }
}
fn group_exists(pgid: u32) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return true;
    };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("stat")).ok())
        .any(|stat| {
            let Some((_, fields)) = stat.rsplit_once(')') else {
                return false;
            };
            let mut fields = fields.split_whitespace();
            let state = fields.next();
            let _ppid = fields.next();
            let group = fields.next().and_then(|v| v.parse::<u32>().ok());
            group == Some(pgid) && !matches!(state, Some("Z" | "X"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_lib_exec::{ArgAtom, ProcessBudget, SealedBindings};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    fn root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sim-platform-process-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn fixture(root: &std::path::Path, program: &str) -> (UbuntuProcess, ProcessRequest) {
        let program_ref = ProgramRef::new("shell").unwrap();
        let root_ref = ProjectRootRef::new("project").unwrap();
        let process = UbuntuProcess::new(
            BTreeMap::from([(program_ref.clone(), PathBuf::from(program))]),
            BTreeMap::from([(root_ref.clone(), root.to_owned())]),
            BTreeMap::new(),
        );
        let request = ProcessRequest {
            program: program_ref,
            argv: vec![],
            root: root_ref,
            environment: SealedBindings::literals([("SIM_PROCESS_VISIBLE".into(), "yes".into())])
                .unwrap(),
            private_artifacts: vec![],
            budget: ProcessBudget {
                timeout_ms: 1_000,
                max_output_bytes: 192,
                stdin: None,
            },
        };
        (process, request)
    }
    #[test]
    fn native_conformance_clears_environment_confines_cwd_and_caps_output() {
        let root = root("success");
        let (process, mut request) = fixture(&root, "/bin/sh");
        request.argv=["-c","printf '%s:%s:' \"$SIM_PROCESS_VISIBLE\" \"${HOME-unset}\"; pwd; printf 12345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890"].into_iter().map(|v|ArgAtom::new(v).unwrap()).collect();
        let ProcessAttempt::Completed { receipt } =
            process.run(&request, &ProcessCancellation::default())
        else {
            panic!("expected completion")
        };
        assert!(receipt.result.stdout.starts_with("yes:unset:"));
        assert!(receipt.result.stdout.contains(root.to_str().unwrap()));
        assert!(receipt.result.truncated);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_conformance_refuses_unknown_refs_and_cleans_timed_out_tree() {
        let root = root("cleanup");
        let (process, mut timed) = fixture(&root, "/bin/sh");
        let mut unknown = timed.clone();
        unknown.program = ProgramRef::new("unknown").unwrap();
        assert!(matches!(
            process.run(&unknown, &ProcessCancellation::default()),
            ProcessAttempt::NotDispatched { .. }
        ));
        timed.argv = ["-c", "sleep 5 & wait"]
            .into_iter()
            .map(|v| ArgAtom::new(v).unwrap())
            .collect();
        timed.budget.timeout_ms = 20;
        let outcome = process.run(&timed, &ProcessCancellation::default());
        assert!(
            matches!(outcome, ProcessAttempt::StoppedAfterTimeout { .. }),
            "{outcome:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_conformance_cancellation_wins_and_cleans_tree() {
        let root = root("cancel");
        let (process, mut request) = fixture(&root, "/bin/sleep");
        request.argv.push(ArgAtom::new("5").unwrap());
        let token = ProcessCancellation::default();
        token.cancel();
        assert!(matches!(
            process.run(&request, &token),
            ProcessAttempt::StoppedAfterCancel { .. }
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
