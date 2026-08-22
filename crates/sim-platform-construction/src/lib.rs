//! Platform-capsule implementations of construction-owned integration ports.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::io::Read;
use std::path::{Path, PathBuf};

use sim_site_dalux::{DaluxHttpRequest, DaluxHttpResponse, DaluxTransport};
use sim_site_powerproject::PowerprojectAutomation;

/// Bounded native HTTP realization for Dalux requests.
#[derive(Clone, Debug)]
pub struct UreqDaluxTransport {
    max_response_bytes: usize,
}

impl UreqDaluxTransport {
    /// Creates a transport with an explicit response-size budget.
    pub fn new(max_response_bytes: usize) -> Result<Self, String> {
        if max_response_bytes == 0 {
            return Err("Dalux response budget must be positive".to_owned());
        }
        Ok(Self { max_response_bytes })
    }
}

impl DaluxTransport for UreqDaluxTransport {
    fn execute(&self, request: &DaluxHttpRequest) -> Result<DaluxHttpResponse, String> {
        let authorization = format!("Bearer {}", request.bearer_token);
        let mut call = ureq::request(request.method, &request.url)
            .set("Accept", "application/json")
            .set("Authorization", &authorization);
        if request.body.is_some() {
            call = call.set("Content-Type", "application/json");
        }
        let response = match &request.body {
            Some(body) => call.send_string(body),
            None => call.call(),
        };
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(error) => return Err(format!("Dalux transport failed: {error}")),
        };
        let status = response.status();
        let mut body = String::new();
        std::io::Read::take(response.into_reader(), self.max_response_bytes as u64 + 1)
            .read_to_string(&mut body)
            .map_err(|error| format!("Dalux response read failed: {error}"))?;
        if body.len() > self.max_response_bytes {
            return Err(format!(
                "Dalux response exceeded {} byte budget",
                self.max_response_bytes
            ));
        }
        Ok(DaluxHttpResponse { status, body })
    }
}

/// Explicitly configured native bridge for Powerproject desktop automation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerprojectBridge {
    executable: PathBuf,
}

impl PowerprojectBridge {
    /// Supplies the already-approved bridge executable; no environment discovery occurs.
    pub fn new(executable: PathBuf) -> Result<Self, String> {
        if executable.as_os_str().is_empty() {
            return Err("Powerproject bridge path is empty".to_owned());
        }
        Ok(Self { executable })
    }
}

impl PowerprojectAutomation for PowerprojectBridge {
    fn export_current_project_to_mspdi(&self, out: &Path) -> Result<(), String> {
        let status = std::process::Command::new(&self.executable)
            .arg("export-current-project-mspdi")
            .arg(out)
            .status()
            .map_err(|error| format!("could not run Powerproject bridge: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("Powerproject bridge exited with {status}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_reject_unbounded_or_missing_configuration() {
        assert!(UreqDaluxTransport::new(0).is_err());
        assert!(PowerprojectBridge::new(PathBuf::new()).is_err());
    }

    #[test]
    fn unavailable_automation_is_a_typed_port_failure() {
        let bridge =
            PowerprojectBridge::new(PathBuf::from("definitely-not-a-real-bridge")).unwrap();
        let error = bridge
            .export_current_project_to_mspdi(Path::new("project.xml"))
            .unwrap_err();
        assert!(error.contains("could not run Powerproject bridge"));
    }
}
