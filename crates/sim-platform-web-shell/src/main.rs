//! Native Linux process capsule for the host-neutral SIM web shell.

#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read},
    process::{Command, ExitCode},
    sync::OnceLock,
    time::{Duration, Instant},
};

use sim_platform_linux::{LinuxDnsPort, LinuxSocketPort};
use sim_transport_ports::TransportServices;
use sim_web_shell::ShellServices;

struct LinuxWebShellServices {
    start: Instant,
}

impl LinuxWebShellServices {
    fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl ShellServices for LinuxWebShellServices {
    fn transport(&self) -> TransportServices {
        TransportServices {
            sockets: std::sync::Arc::new(LinuxSocketPort),
            dns: std::sync::Arc::new(LinuxDnsPort),
            ipc: None,
        }
    }

    fn read_mount(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn monotonic(&self) -> Duration {
        self.start.elapsed()
    }

    fn fill_entropy(&self, bytes: &mut [u8]) -> io::Result<()> {
        File::open("/dev/urandom")?.read_exact(bytes)
    }

    fn open_external(&self, url: &str) -> io::Result<()> {
        let status = Command::new("xdg-open").arg(url).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("xdg-open exited with {status}")))
        }
    }
}

fn main() -> ExitCode {
    static SERVICES: OnceLock<std::sync::Arc<LinuxWebShellServices>> = OnceLock::new();
    let services = SERVICES
        .get_or_init(|| std::sync::Arc::new(LinuxWebShellServices::new()))
        .clone();
    let mut args: Vec<OsString> = ["sim-web-shell", "--codec", "lisp", "serve"]
        .into_iter()
        .map(OsString::from)
        .collect();
    args.extend(std::env::args_os().skip(1));
    match sim_web_shell::web_bootloader_with_services(services).run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("sim-web-shell: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn linux_capsule_realizes_declared_shell_services() {
        let services = LinuxWebShellServices::new();
        assert!(services.monotonic() >= Duration::ZERO);
        let mut entropy = [0_u8; 16];
        services.fill_entropy(&mut entropy).unwrap();
        assert_ne!(entropy, [0; 16]);

        let root =
            std::env::temp_dir().join(format!("sim-platform-web-shell-{}", std::process::id()));
        let mut file = std::fs::File::create(&root).unwrap();
        file.write_all(b"mounted").unwrap();
        assert_eq!(
            services.read_mount(root.to_str().unwrap()).unwrap(),
            b"mounted"
        );
        std::fs::remove_file(root).unwrap();
    }
}
