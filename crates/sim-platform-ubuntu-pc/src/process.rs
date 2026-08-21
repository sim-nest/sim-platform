use sim_lib_exec::{
    ProcResult, ProcessCancellation, ProcessError, ProcessPort, ProcessReceipt, ProcessRequest,
};
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt as _,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

/// Ubuntu process adapter. All native process mechanics are confined here.
#[derive(Clone, Copy, Debug, Default)]
pub struct UbuntuProcess;
impl ProcessPort for UbuntuProcess {
    fn run(
        &self,
        request: &ProcessRequest,
        cancellation: &ProcessCancellation,
    ) -> Result<ProcessReceipt, ProcessError> {
        let root = request.root.canonicalize().map_err(|e| {
            ProcessError::Confinement(format!("root {}: {e}", request.root.display()))
        })?;
        let cwd = request.cwd.canonicalize().map_err(|e| {
            ProcessError::Confinement(format!("cwd {}: {e}", request.cwd.display()))
        })?;
        if !cwd.starts_with(&root) {
            return Err(ProcessError::Confinement(format!(
                "cwd {} escapes root {}",
                cwd.display(),
                root.display()
            )));
        }
        let mut command = Command::new(&request.argv[0]);
        command
            .args(&request.argv[1..])
            .current_dir(cwd)
            .env_clear()
            .envs(&request.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| ProcessError::Spawn(format!("{}: {e}", request.argv[0])))?;
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
fn run_child(
    child: &mut Child,
    request: &ProcessRequest,
    cancellation: &ProcessCancellation,
) -> Result<ProcessReceipt, ProcessError> {
    let started = Instant::now();
    let deadline = started
        .checked_add(Duration::from_millis(request.timeout_ms))
        .ok_or_else(|| ProcessError::Io("timeout overflow".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ProcessError::Io("stdout pipe missing".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ProcessError::Io("stderr pipe missing".into()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| ProcessError::Io("stdin pipe missing".into()))?;
    let budget = Arc::new(Mutex::new(Budget {
        left: request.max_output_bytes,
        truncated: false,
    }));
    let (tx, rx) = mpsc::channel();
    reader(stdout, Arc::clone(&budget), tx.clone(), true);
    reader(stderr, Arc::clone(&budget), tx.clone(), false);
    writer(stdin, request.stdin.clone(), tx.clone());
    drop(tx);
    let (mut status, mut out, mut err, mut input) = (None, None, None, None);
    loop {
        if status.is_none() {
            status = child
                .try_wait()
                .map_err(|e| ProcessError::Io(format!("wait: {e}")))?;
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
            input.map_err(ProcessError::Io)?;
            let out = out.map_err(ProcessError::Io)?;
            let err = err.map_err(ProcessError::Io)?;
            let truncated = budget
                .lock()
                .map_err(|_| ProcessError::Io("output budget poisoned".into()))?
                .truncated;
            let elapsed_mono_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            return Ok(ProcessReceipt {
                provider: "platform/site/ubuntu-pc".into(),
                elapsed_mono_ns,
                result: ProcResult {
                    stdout: String::from_utf8_lossy(&out).into_owned(),
                    stderr: String::from_utf8_lossy(&err).into_owned(),
                    exit_code: status.code().unwrap_or(-1),
                    truncated,
                },
            });
        }
        let cancelled = cancellation.is_cancelled();
        if cancelled || Instant::now() >= deadline {
            let (kill_failure, leaked_descendants) = terminate_tree(child);
            return Err(if cancelled {
                ProcessError::Cancelled {
                    kill_failure,
                    leaked_descendants,
                }
            } else {
                ProcessError::Timeout {
                    kill_failure,
                    leaked_descendants,
                }
            });
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
fn terminate_tree(child: &mut Child) -> (Option<String>, bool) {
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
    let failure = if leaked {
        term.err().or_else(|| kill.err()).or_else(|| wait.err())
    } else {
        wait.err()
    };
    (failure, leaked)
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
    use std::{
        collections::BTreeMap,
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
        fs::create_dir_all(path.join("work")).unwrap();
        path
    }
    fn request(root: &std::path::Path, argv: Vec<String>, budget: usize) -> ProcessRequest {
        ProcessRequest {
            argv,
            cwd: root.join("work"),
            root: root.to_owned(),
            timeout_ms: 1_000,
            max_output_bytes: budget,
            stdin: None,
            environment: BTreeMap::from([("SIM_PROCESS_VISIBLE".into(), "yes".into())]),
        }
    }
    #[test]
    fn native_conformance_clears_environment_confines_cwd_and_caps_output() {
        let root = root("success");
        let argv = vec!["sh".into(), "-c".into(), "printf '%s:%s:' \"$SIM_PROCESS_VISIBLE\" \"${HOME-unset}\"; pwd; printf 12345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890".into()];
        let receipt = UbuntuProcess
            .run(&request(&root, argv, 192), &ProcessCancellation::default())
            .unwrap();
        assert!(receipt.result.stdout.starts_with("yes:unset:"));
        assert!(
            receipt
                .result
                .stdout
                .contains(root.join("work").to_str().unwrap())
        );
        assert!(receipt.result.truncated);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_conformance_rejects_cwd_escape_and_cleans_timed_out_tree() {
        let root = root("cleanup");
        let outside = root.parent().unwrap().to_owned();
        let mut escaped = request(&root, vec!["true".into()], 10);
        escaped.cwd = outside;
        assert!(matches!(
            UbuntuProcess.run(&escaped, &ProcessCancellation::default()),
            Err(ProcessError::Confinement(_))
        ));
        let mut timed = request(
            &root,
            vec!["sh".into(), "-c".into(), "sleep 5 & wait".into()],
            10,
        );
        timed.timeout_ms = 20;
        let outcome = UbuntuProcess.run(&timed, &ProcessCancellation::default());
        assert!(
            matches!(
                outcome,
                Err(ProcessError::Timeout {
                    kill_failure: None,
                    leaked_descendants: false
                })
            ),
            "{outcome:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_conformance_cancellation_wins_and_cleans_tree() {
        let root = root("cancel");
        let token = ProcessCancellation::default();
        token.cancel();
        let request = request(&root, vec!["sleep".into(), "5".into()], 10);
        assert!(matches!(
            UbuntuProcess.run(&request, &token),
            Err(ProcessError::Cancelled {
                kill_failure: None,
                leaked_descendants: false
            })
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
