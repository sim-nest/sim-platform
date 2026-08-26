//! Native Linux process capsule for the host-neutral SIM web shell.

#![forbid(unsafe_code)]

// conformance: the native web shell supplies only declared host-neutral services.

use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read},
    process::{Command, ExitCode},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use sim_platform_linux::{LinuxDnsPort, LinuxSocketPort};
use sim_run_core::{Bootloader, RuntimeConfigState};
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
    static SERVICES: OnceLock<Arc<LinuxWebShellServices>> = OnceLock::new();
    let services = SERVICES
        .get_or_init(|| Arc::new(LinuxWebShellServices::new()))
        .clone();
    let mut args: Vec<OsString> = ["sim-web-shell", "--codec", "lisp", "serve"]
        .into_iter()
        .map(OsString::from)
        .collect();
    args.extend(std::env::args_os().skip(1));
    let bootloader = sim_web_shell::configure_web_bootloader_with_cookbook_and_services(
        Bootloader::standard().with_context(install_web_minimum_loaded),
        vec![sim_lib_cookbook::cookbook_lib_symbol()],
        Arc::new(cookbook_web_state),
        services,
    );
    match bootloader.run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!("sim-web-shell: {error}");
            ExitCode::from(2)
        }
    }
}

fn install_web_minimum_loaded(cx: &mut sim::kernel::Cx) {
    cx.set_eval_policy(Arc::new(sim::kernel::EagerPolicy));
    sim::runtime::install_core_runtime(cx);
}

fn cookbook_web_state(config: &RuntimeConfigState) -> sim_lib_server::CookbookWebState {
    let provider = sim_lib_cookbook::ConfigCookbookProvider::new_with_base(
        config.effective(),
        sim::runtime::cookbook_directory::default_cookbook_config(),
        &sim::runtime::cookbook_directory::SimNestCookbookResolver,
    );
    let (directory, mut diagnostics) = provider.loadable_libs();
    diagnostics.extend(config.diagnostics().iter().cloned());
    sim_lib_server::CookbookWebState::from_loadable_libs(directory, diagnostics)
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
