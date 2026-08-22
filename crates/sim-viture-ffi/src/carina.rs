//! Safe wrappers for VITURE Carina provider entry points.

use std::ffi::c_void;
use std::fmt;
use std::ptr::NonNull;
use std::sync::Arc;

use libloading::Library;

use crate::dynload::{VitureError, VitureLib, VitureResult, VitureStatus};

type ProviderCreateFn = unsafe extern "C" fn() -> *mut c_void;
type ProviderControlFn = unsafe extern "C" fn(*mut c_void) -> i32;
type ProviderDestroyFn = unsafe extern "C" fn(*mut c_void);
type CarinaPoseFn = unsafe extern "C" fn(*mut c_void, u64, *mut f64) -> i32;

const CREATE: &str = "xr_device_provider_create";
const INITIALIZE: &str = "xr_device_provider_initialize";
const START: &str = "xr_device_provider_start";
const STOP: &str = "xr_device_provider_stop";
const DESTROY: &str = "xr_device_provider_destroy";
const CARINA_POSE: &str = "xr_device_provider_get_gl_pose_carina";

/// Safe Carina pose return value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarinaPose {
    /// Pose as `[px, py, pz, qw, qx, qy, qz]`.
    pub pose: [f64; 7],
    /// Successful SDK status.
    pub status: VitureStatus,
}

/// Opaque VITURE provider handle.
pub struct VitureHandle {
    raw: NonNull<c_void>,
    library: Arc<Library>,
}

impl VitureHandle {
    pub(crate) fn new(raw: NonNull<c_void>, library: Arc<Library>) -> Self {
        Self { raw, library }
    }

    fn raw(&self) -> *mut c_void {
        self.raw.as_ptr()
    }
}

impl fmt::Debug for VitureHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VitureHandle").finish_non_exhaustive()
    }
}

// SAFETY: the handle is opaque and only moved, not accessed concurrently, through
// the safe session wrappers that own it. The vendor library remains alive through
// the internal `Arc<Library>`.
unsafe impl Send for VitureHandle {}

impl Drop for VitureHandle {
    fn drop(&mut self) {
        if let Ok(destroy) = VitureLib::symbol::<ProviderDestroyFn>(
            &self.library,
            DESTROY,
            b"xr_device_provider_destroy\0",
        ) {
            // SAFETY: the handle was returned by `xr_device_provider_create` from
            // the same loaded library and has not been exposed outside this type.
            unsafe { destroy(self.raw()) };
        }
    }
}

impl VitureLib {
    /// Creates an opaque Carina provider handle.
    pub fn open_carina(&self) -> VitureResult<VitureHandle> {
        let library = self.dynamic_library()?.clone();
        let raw = {
            let create =
                Self::symbol::<ProviderCreateFn>(&library, CREATE, b"xr_device_provider_create\0")?;
            // SAFETY: the loaded symbol has the SDK provider-create ABI. A null
            // pointer is rejected before building the safe handle.
            unsafe { create() }
        };
        let raw = NonNull::new(raw).ok_or(VitureError::NullHandle)?;
        Ok(VitureHandle::new(raw, library))
    }

    /// Initializes a Carina provider handle.
    pub fn initialize_carina(&self, handle: &VitureHandle) -> VitureResult<VitureStatus> {
        provider_control(
            &handle.library,
            INITIALIZE,
            b"xr_device_provider_initialize\0",
            handle,
        )
    }

    /// Starts a Carina provider handle.
    pub fn start_carina(&self, handle: &VitureHandle) -> VitureResult<VitureStatus> {
        provider_control(
            &handle.library,
            START,
            b"xr_device_provider_start\0",
            handle,
        )
    }

    /// Stops a Carina provider handle.
    pub fn stop_carina(&self, handle: &VitureHandle) -> VitureResult<VitureStatus> {
        provider_control(&handle.library, STOP, b"xr_device_provider_stop\0", handle)
    }

    /// Reads a predicted Carina glasses pose.
    pub fn carina_pose(&self, handle: &VitureHandle, predict_ns: u64) -> VitureResult<CarinaPose> {
        let pose = Self::symbol::<CarinaPoseFn>(
            &handle.library,
            CARINA_POSE,
            b"xr_device_provider_get_gl_pose_carina\0",
        )?;
        let mut out = [0.0f64; 7];
        // SAFETY: `handle` owns a non-null SDK handle from the same library and
        // `out` points to seven writable f64 slots for the SDK to fill.
        let status =
            VitureStatus::from_code(unsafe { pose(handle.raw(), predict_ns, out.as_mut_ptr()) })?;
        Ok(CarinaPose { pose: out, status })
    }
}

fn provider_control(
    library: &Library,
    name: &'static str,
    bytes: &'static [u8],
    handle: &VitureHandle,
) -> VitureResult<VitureStatus> {
    let function = VitureLib::symbol::<ProviderControlFn>(library, name, bytes)?;
    // SAFETY: `handle` owns a non-null provider handle from the same library, and
    // the symbol is used with the SDK one-handle status-return ABI.
    VitureStatus::from_code(unsafe { function(handle.raw()) })
}
