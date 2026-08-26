//! Native compute discovery owned by the Ubuntu platform capsule.

use std::{process::Command, sync::mpsc};

use sim_lib_compute_auto::{ComputeDeviceIdentity, ComputeEvidenceKind};
use sim_lib_compute_cuda::{
    CudaLoadError, CudaProbePort, CudaRuntimeLoader, CudaRuntimeProbe, DynamicCudaLoader,
};
use sim_lib_compute_rocm::{
    DynamicRocmLoader, RocmLoadError, RocmProbePort, RocmRuntimeLoader, RocmRuntimeProbe,
};
use sim_lib_compute_wgpu::{
    AllocationAttempt, ProbeEvidence, ProbePolicy, RequestedWgpuProfile, TransferEvidence,
    WgpuAdapterEvidence, WgpuAdapterProbe, WgpuAdapterRuntime, WgpuCapabilityEvidence,
    WgpuDiscoveryError, WgpuLimitEvidence, WgpuProbePort,
};
use wgpu::{
    BufferDescriptor, BufferUsages, DeviceDescriptor, ExperimentalFeatures, Features, Instance,
    Limits, MapMode, MemoryHints, PollType,
};

/// Ubuntu implementation of the compute probe membrane.
#[derive(Clone, Debug)]
pub struct UbuntuComputeProbe {
    cuda: CudaRuntimeLoader,
    rocm: RocmRuntimeLoader,
}

impl UbuntuComputeProbe {
    /// Uses Ubuntu's normal native library and adapter search policy.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cuda: CudaRuntimeLoader::new(),
            rocm: RocmRuntimeLoader::new().with_observed_gfx_targets(observed_gfx_targets()),
        }
    }
}

impl Default for UbuntuComputeProbe {
    fn default() -> Self {
        Self::new()
    }
}

fn observed_gfx_targets() -> Vec<String> {
    let Ok(output) = Command::new("rocm_agent_enumerator").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(stdout) = String::from_utf8(output.stdout) else {
        return Vec::new();
    };
    parse_gfx_targets(&stdout)
}

fn parse_gfx_targets(stdout: &str) -> Vec<String> {
    let mut targets = stdout
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("gfx") && *line != "gfx000")
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    targets
}

impl CudaProbePort for UbuntuComputeProbe {
    fn probe_cuda(&self) -> Result<CudaRuntimeProbe, CudaLoadError> {
        self.cuda.discover()
    }
}

impl RocmProbePort for UbuntuComputeProbe {
    fn probe_rocm(&self) -> Result<RocmRuntimeProbe, RocmLoadError> {
        self.rocm.discover()
    }
}

impl WgpuProbePort for UbuntuComputeProbe {
    fn probe_wgpu(
        &self,
        policy: &ProbePolicy,
    ) -> Result<Vec<WgpuAdapterRuntime>, WgpuDiscoveryError> {
        let instance = Instance::default();
        let adapters = pollster::block_on(instance.enumerate_adapters(policy.backends));
        let mut runtimes = adapters
            .into_iter()
            .filter_map(|adapter| probe_adapter(&adapter, policy).ok())
            .collect::<Vec<_>>();
        runtimes.sort_by(|left, right| {
            left.probe()
                .adapter
                .sort_key()
                .cmp(&right.probe().adapter.sort_key())
        });
        Ok(runtimes)
    }
}

fn probe_adapter(
    adapter: &wgpu::Adapter,
    policy: &ProbePolicy,
) -> Result<WgpuAdapterRuntime, WgpuDiscoveryError> {
    let info = adapter.get_info();
    let backend = format!("{:?}", info.backend);
    let identity = ComputeDeviceIdentity::new(info.name.clone(), "wgpu", backend.clone());
    let required_features = adapter.features() & (Features::TIMESTAMP_QUERY | Features::SHADER_F16);
    let required_limits = Limits::downlevel_defaults().using_resolution(adapter.limits());
    let descriptor = DeviceDescriptor {
        label: Some("sim-platform-ubuntu-compute-probe"),
        required_features,
        required_limits: required_limits.clone(),
        experimental_features: ExperimentalFeatures::disabled(),
        memory_hints: MemoryHints::Performance,
        trace: wgpu::Trace::default(),
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor))
        .map_err(|error| WgpuDiscoveryError::new(format!("wgpu request_device failed: {error}")))?;
    let bytes = policy.transfer_bytes.max(4).next_multiple_of(4);
    let payload = (0..bytes)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let source = device.create_buffer(&BufferDescriptor {
        label: Some("sim-platform-wgpu-probe"),
        size: bytes,
        usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("sim-platform-wgpu-readback-probe"),
        size: bytes,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    queue.write_buffer(&source, 0, &payload);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("sim-platform-wgpu-transfer-probe"),
    });
    encoder.copy_buffer_to_buffer(&source, 0, &readback, 0, bytes);
    queue.submit([encoder.finish()]);
    let (sender, receiver) = mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(PollType::wait_indefinitely())
        .map_err(|error| WgpuDiscoveryError::new(format!("wgpu poll failed: {error}")))?;
    receiver
        .recv()
        .map_err(|error| WgpuDiscoveryError::new(format!("wgpu map callback failed: {error}")))?
        .map_err(|error| WgpuDiscoveryError::new(format!("wgpu map failed: {error}")))?;
    let mapping_ok = readback
        .slice(..)
        .get_mapped_range()
        .map_err(|error| WgpuDiscoveryError::new(format!("wgpu mapped range failed: {error}")))?
        .as_ref()
        == payload.as_slice();
    readback.unmap();
    let ceiling = device
        .limits()
        .max_buffer_size
        .min(policy.max_allocation_probe_bytes)
        .max(4);
    let allocation = device.create_buffer(&BufferDescriptor {
        label: Some("sim-platform-wgpu-allocation-probe"),
        size: ceiling,
        usage: BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    drop(allocation);
    let probe = WgpuAdapterProbe {
        evidence_kind: ComputeEvidenceKind::PhysicalDevice,
        claimed_identity: Some(identity.clone()),
        observed_identity: Some(identity),
        adapter: WgpuAdapterEvidence {
            ordinal: 0,
            name: info.name,
            backend,
            adapter_type: format!("{:?}", info.device_type),
            vendor: info.vendor,
            device: info.device,
            requested: RequestedWgpuProfile::from_parts(required_limits, required_features),
            granted_limits: WgpuLimitEvidence::from_limits(&device.limits()),
            granted_features: WgpuCapabilityEvidence::from_features(device.features()),
        },
        probe: ProbeEvidence {
            transfer: TransferEvidence {
                bytes,
                transfer_ok: true,
                mapping_ok,
            },
            allocation_attempts: vec![AllocationAttempt {
                bytes: ceiling,
                success: true,
            }],
        },
    };
    Ok(WgpuAdapterRuntime::new(probe, device, queue))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_lib_compute_cuda::ComputeCudaLib;
    use sim_lib_compute_rocm::ComputeRocmLib;
    use sim_lib_compute_wgpu::ComputeWgpuLib;

    struct EmptyWgpuProbe;

    impl WgpuProbePort for EmptyWgpuProbe {
        fn probe_wgpu(
            &self,
            _policy: &ProbePolicy,
        ) -> Result<Vec<WgpuAdapterRuntime>, WgpuDiscoveryError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn amd_target_observation_is_filtered_sorted_and_deduplicated() {
        assert_eq!(
            parse_gfx_targets("gfx1151\ngfx000\nnoise\ngfx1103\ngfx1151\n"),
            vec!["gfx1103", "gfx1151"]
        );
    }

    #[test]
    fn absent_native_runtimes_export_no_provider() {
        let probe = UbuntuComputeProbe {
            cuda: CudaRuntimeLoader::with_search_dirs_only(Vec::new()),
            rocm: RocmRuntimeLoader::with_search_dirs_only(Vec::new()),
        };
        assert!(ComputeCudaLib::from_probe_port(&probe).is_err());
        assert!(ComputeRocmLib::from_probe_port(&probe).is_err());
        let wgpu =
            ComputeWgpuLib::from_probe_port(&EmptyWgpuProbe, &ProbePolicy::default()).unwrap();
        assert!(sim_kernel::Lib::manifest(&wgpu).exports.is_empty());
    }
}
