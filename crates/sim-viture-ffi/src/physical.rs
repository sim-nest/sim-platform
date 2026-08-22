//! Narrow physical-device lifecycle used by stream-facing VITURE adapters.

use std::collections::VecDeque;

use crate::{CarinaPose, LegacyImuRate, VitureError, VitureHandle, VitureLib, VitureResult};

/// A vendor-neutral sample delivered by the physical port.
#[derive(Clone, Debug, PartialEq)]
pub enum PhysicalDeviceSample {
    /// A pose consisting of position followed by orientation.
    Pose {
        /// Position xyz followed by orientation xyzw.
        pose: [f64; 7],
        /// Whether the native tracker reports full tracking.
        tracked: bool,
    },
}

/// Commands required by the delivered glasses session contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalDeviceCommand {
    /// Enable or disable IMU reports.
    ImuReports(bool),
    /// Select an IMU report rate.
    ImuRate(LegacyImuRate),
    /// Enable or disable stereoscopic display mode.
    Display3d(bool),
}

/// Physical transport factory. Discovery and native loading end at this seam.
pub trait DevicePhysicalPort: Send + Sync {
    /// Connects one independently owned physical session.
    fn connect(&self) -> VitureResult<Box<dyn DevicePhysicalSession>>;
}

/// Connected physical session with explicit reconnect and cleanup semantics.
pub trait DevicePhysicalSession: Send {
    /// Starts device traffic.
    fn start(&mut self) -> VitureResult<()>;
    /// Re-establishes the connection after a transport loss.
    fn reconnect(&mut self) -> VitureResult<()>;
    /// Polls one sample without imposing stream semantics.
    fn poll(&mut self) -> VitureResult<Option<PhysicalDeviceSample>>;
    /// Sends one bounded physical command.
    fn send(&mut self, command: PhysicalDeviceCommand) -> VitureResult<()>;
    /// Stops traffic and releases native resources.
    fn close(&mut self) -> VitureResult<()>;
    /// Returns transport-local queue and drop accounting.
    fn stats(&self) -> PhysicalTransportStats;
}

/// Observable bounded-queue accounting for a physical session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicalTransportStats {
    /// Samples still queued for delivery.
    pub queued: usize,
    /// Samples dropped at the configured queue boundary.
    pub dropped: u64,
}

/// Ubuntu-supported VITURE SDK transport.
#[derive(Clone, Debug)]
pub struct ViturePhysicalPort {
    lib: VitureLib,
    route: PhysicalRoute,
}

#[derive(Clone, Copy, Debug)]
enum PhysicalRoute {
    Carina { predict_ns: u64 },
    Legacy { rate: LegacyImuRate },
}

impl ViturePhysicalPort {
    /// Builds a transport from an explicitly loaded SDK and prediction horizon.
    pub fn new(lib: VitureLib, predict_ns: u64) -> Self {
        Self {
            lib,
            route: PhysicalRoute::Carina { predict_ns },
        }
    }

    /// Builds the legacy IMU transport.
    pub fn legacy(lib: VitureLib, rate: LegacyImuRate) -> Self {
        Self {
            lib,
            route: PhysicalRoute::Legacy { rate },
        }
    }
}

impl DevicePhysicalPort for ViturePhysicalPort {
    fn connect(&self) -> VitureResult<Box<dyn DevicePhysicalSession>> {
        let route = match self.route {
            PhysicalRoute::Carina { predict_ns } => PhysicalSessionRoute::Carina {
                handle: self.lib.open_carina()?,
                predict_ns,
            },
            PhysicalRoute::Legacy { rate } => {
                self.lib.legacy_init()?;
                PhysicalSessionRoute::Legacy { rate }
            }
        };
        Ok(Box::new(ViturePhysicalSession {
            lib: self.lib.clone(),
            route,
            started: false,
        }))
    }
}

struct ViturePhysicalSession {
    lib: VitureLib,
    route: PhysicalSessionRoute,
    started: bool,
}

enum PhysicalSessionRoute {
    Carina {
        handle: VitureHandle,
        predict_ns: u64,
    },
    Legacy {
        rate: LegacyImuRate,
    },
}

impl DevicePhysicalSession for ViturePhysicalSession {
    fn start(&mut self) -> VitureResult<()> {
        match &self.route {
            PhysicalSessionRoute::Carina { handle, .. } => {
                self.lib.initialize_carina(handle)?;
                self.lib.start_carina(handle)?;
            }
            PhysicalSessionRoute::Legacy { rate } => {
                self.lib.legacy_set_imu_fq(*rate)?;
                self.lib.legacy_set_imu(true)?;
            }
        }
        self.started = true;
        Ok(())
    }

    fn reconnect(&mut self) -> VitureResult<()> {
        if self.started {
            self.close()?;
        }
        if let PhysicalSessionRoute::Carina { handle, .. } = &mut self.route {
            *handle = self.lib.open_carina()?;
        }
        self.start()
    }

    fn poll(&mut self) -> VitureResult<Option<PhysicalDeviceSample>> {
        if !self.started {
            return Err(VitureError::Unsupported);
        }
        match &self.route {
            PhysicalSessionRoute::Carina { handle, predict_ns } => {
                let CarinaPose { pose, status } = self.lib.carina_pose(handle, *predict_ns)?;
                Ok(Some(PhysicalDeviceSample::Pose {
                    pose,
                    tracked: status.code() == 0,
                }))
            }
            PhysicalSessionRoute::Legacy { .. } => Ok(None),
        }
    }

    fn send(&mut self, command: PhysicalDeviceCommand) -> VitureResult<()> {
        match command {
            PhysicalDeviceCommand::ImuReports(enabled) => self.lib.legacy_set_imu(enabled)?,
            PhysicalDeviceCommand::ImuRate(rate) => self.lib.legacy_set_imu_fq(rate)?,
            PhysicalDeviceCommand::Display3d(enabled) => self.lib.legacy_set_3d(enabled)?,
        };
        Ok(())
    }

    fn close(&mut self) -> VitureResult<()> {
        if self.started {
            match &self.route {
                PhysicalSessionRoute::Carina { handle, .. } => {
                    self.lib.stop_carina(handle)?;
                }
                PhysicalSessionRoute::Legacy { .. } => {
                    self.lib.legacy_set_imu(false)?;
                }
            }
            self.started = false;
        }
        Ok(())
    }

    fn stats(&self) -> PhysicalTransportStats {
        PhysicalTransportStats::default()
    }
}

/// Deterministic physical transport used for lifecycle and budget tests.
#[derive(Clone, Debug)]
pub struct ModeledViturePort {
    samples: Vec<PhysicalDeviceSample>,
    dropped: u64,
}

impl ModeledViturePort {
    /// Builds a finite modeled transport.
    pub fn new(samples: Vec<PhysicalDeviceSample>) -> Self {
        Self {
            samples,
            dropped: 0,
        }
    }

    /// Builds a modeled transport with an explicit backpressure capacity.
    pub fn with_capacity(mut samples: Vec<PhysicalDeviceSample>, capacity: usize) -> Self {
        let dropped = samples.len().saturating_sub(capacity) as u64;
        if dropped != 0 {
            samples.drain(..dropped as usize);
        }
        Self { samples, dropped }
    }
}

impl DevicePhysicalPort for ModeledViturePort {
    fn connect(&self) -> VitureResult<Box<dyn DevicePhysicalSession>> {
        Ok(Box::new(ModeledSession {
            original: self.samples.clone(),
            samples: self.samples.clone().into(),
            started: false,
            dropped: self.dropped,
        }))
    }
}

struct ModeledSession {
    original: Vec<PhysicalDeviceSample>,
    samples: VecDeque<PhysicalDeviceSample>,
    started: bool,
    dropped: u64,
}

impl DevicePhysicalSession for ModeledSession {
    fn start(&mut self) -> VitureResult<()> {
        self.started = true;
        Ok(())
    }
    fn reconnect(&mut self) -> VitureResult<()> {
        self.samples = self.original.clone().into();
        self.started = true;
        Ok(())
    }
    fn poll(&mut self) -> VitureResult<Option<PhysicalDeviceSample>> {
        if !self.started {
            return Err(VitureError::Unsupported);
        }
        Ok(self.samples.pop_front())
    }
    fn send(&mut self, _command: PhysicalDeviceCommand) -> VitureResult<()> {
        if !self.started {
            return Err(VitureError::Unsupported);
        }
        Ok(())
    }
    fn close(&mut self) -> VitureResult<()> {
        self.started = false;
        self.samples.clear();
        Ok(())
    }
    fn stats(&self) -> PhysicalTransportStats {
        PhysicalTransportStats {
            queued: self.samples.len(),
            dropped: self.dropped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modeled_transport_connects_reconnects_and_cleans_up() {
        let sample = PhysicalDeviceSample::Pose {
            pose: [0.0; 7],
            tracked: true,
        };
        let mut session = ModeledViturePort::new(vec![sample.clone()])
            .connect()
            .unwrap();
        assert!(session.poll().is_err());
        session.start().unwrap();
        assert_eq!(session.poll().unwrap(), Some(sample.clone()));
        assert_eq!(session.poll().unwrap(), None);
        session.reconnect().unwrap();
        assert_eq!(session.poll().unwrap(), Some(sample));
        session.close().unwrap();
        assert!(session.poll().is_err());
    }

    #[test]
    fn modeled_transport_applies_backpressure_and_counts_drops() {
        let sample = PhysicalDeviceSample::Pose {
            pose: [0.0; 7],
            tracked: true,
        };
        let mut session =
            ModeledViturePort::with_capacity(vec![sample.clone(), sample.clone(), sample], 2)
                .connect()
                .unwrap();
        assert_eq!(
            session.stats(),
            PhysicalTransportStats {
                queued: 2,
                dropped: 1
            }
        );
        session.start().unwrap();
        session.poll().unwrap();
        assert_eq!(
            session.stats(),
            PhysicalTransportStats {
                queued: 1,
                dropped: 1
            }
        );
    }
}
