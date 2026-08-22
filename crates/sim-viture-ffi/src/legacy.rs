//! Wrappers for the IMU-oriented VITURE SDK entry points.

use crate::dynload::{VitureLib, VitureResult, VitureStatus};

type InitFn = unsafe extern "C" fn() -> i32;
type SetFlagFn = unsafe extern "C" fn(i32) -> i32;
type SetFrequencyFn = unsafe extern "C" fn(u32) -> i32;

const INIT: &str = "init";
const SET_IMU: &str = "set_imu";
const SET_3D: &str = "set_3d";
const SET_IMU_FQ: &str = "set_imu_fq";

/// Non-zero IMU sample frequency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyImuRate {
    hz: u32,
}

impl LegacyImuRate {
    /// Builds a non-zero IMU sample frequency.
    pub fn new(hz: u32) -> Option<Self> {
        (hz > 0).then_some(Self { hz })
    }

    /// Returns the frequency in hertz.
    pub const fn hz(self) -> u32 {
        self.hz
    }
}

impl VitureLib {
    /// Initializes the IMU-oriented SDK surface.
    pub fn legacy_init(&self) -> VitureResult<VitureStatus> {
        let library = self.dynamic_library()?;
        let init = Self::symbol::<InitFn>(library, INIT, b"init\0")?;
        // SAFETY: the loaded symbol is called with its zero-argument status ABI.
        VitureStatus::from_code(unsafe { init() })
    }

    /// Enables or disables IMU reports.
    pub fn legacy_set_imu(&self, enabled: bool) -> VitureResult<VitureStatus> {
        self.legacy_set_flag(SET_IMU, b"set_imu\0", enabled)
    }

    /// Enables or disables 3D display mode.
    pub fn legacy_set_3d(&self, enabled: bool) -> VitureResult<VitureStatus> {
        self.legacy_set_flag(SET_3D, b"set_3d\0", enabled)
    }

    /// Sets the IMU sample frequency.
    pub fn legacy_set_imu_fq(&self, rate: LegacyImuRate) -> VitureResult<VitureStatus> {
        let library = self.dynamic_library()?;
        let set = Self::symbol::<SetFrequencyFn>(library, SET_IMU_FQ, b"set_imu_fq\0")?;
        // SAFETY: the loaded symbol is called with the SDK frequency status ABI.
        VitureStatus::from_code(unsafe { set(rate.hz()) })
    }

    fn legacy_set_flag(
        &self,
        name: &'static str,
        bytes: &'static [u8],
        enabled: bool,
    ) -> VitureResult<VitureStatus> {
        let library = self.dynamic_library()?;
        let set = Self::symbol::<SetFlagFn>(library, name, bytes)?;
        // SAFETY: the loaded symbol is called with the SDK boolean-as-int status ABI.
        VitureStatus::from_code(unsafe { set(i32::from(enabled)) })
    }
}
