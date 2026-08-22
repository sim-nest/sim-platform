use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    LegacyImuRate, SdkDiscoverySource, VITURE_USB_VID, VitureError, VitureLib, VitureSdkDiscovery,
    unsupported_viture_lib,
};

#[test]
fn no_sdk_returns_unsupported() {
    let lib = unsupported_viture_lib();
    assert_eq!(lib.open_carina().unwrap_err(), VitureError::Unsupported);
    assert_eq!(lib.legacy_init().unwrap_err(), VitureError::Unsupported);
    assert_eq!(
        lib.legacy_set_imu(true).unwrap_err(),
        VitureError::Unsupported
    );
    assert_eq!(
        lib.legacy_set_3d(false).unwrap_err(),
        VitureError::Unsupported
    );
    assert_eq!(
        lib.legacy_set_imu_fq(LegacyImuRate::new(120).unwrap())
            .unwrap_err(),
        VitureError::Unsupported
    );
}

#[test]
fn discovery_lists_config_runtime_and_linux_vid_candidates() {
    let sysfs = temp_sysfs_root();
    let device = sysfs.join("1-1");
    let other = sysfs.join("2-1");
    fs::create_dir_all(&device).unwrap();
    fs::create_dir_all(&other).unwrap();
    fs::write(device.join("idVendor"), "35CA\n").unwrap();
    fs::write(other.join("idVendor"), "1234\n").unwrap();

    let configured = PathBuf::from("/opt/viture/libviture_sdk.so");
    let discovery = VitureSdkDiscovery::new()
        .without_runtime_names()
        .with_configured_path(configured.clone())
        .with_runtime_name("libcustom_viture.so")
        .with_sysfs_root(sysfs.clone());
    let candidates = discovery.candidates();

    assert_eq!(VITURE_USB_VID, "0x35CA");
    assert_eq!(candidates.len(), 3);
    assert_eq!(candidates[0].source(), SdkDiscoverySource::ConfiguredPath);
    assert_eq!(candidates[0].library_path(), Some(configured.as_path()));
    assert_eq!(candidates[1].source(), SdkDiscoverySource::RuntimeLinker);
    assert_eq!(candidates[1].linker_name().unwrap(), "libcustom_viture.so");
    assert_eq!(candidates[2].source(), SdkDiscoverySource::LinuxSysfsVid);
    assert_eq!(candidates[2].sysfs_device(), Some(device.as_path()));

    fs::remove_dir_all(sysfs).unwrap();
}

#[test]
fn discovery_without_loadable_sdk_yields_stub() {
    let sysfs = temp_sysfs_root();
    let lib = VitureLib::discover(
        &VitureSdkDiscovery::new()
            .without_runtime_names()
            .with_sysfs_root(sysfs.clone()),
    )
    .unwrap();

    assert_eq!(lib.open_carina().unwrap_err(), VitureError::Unsupported);
    fs::remove_dir_all(sysfs).unwrap();
}

#[test]
fn legacy_rate_rejects_zero_frequency() {
    assert_eq!(LegacyImuRate::new(0), None);
    assert_eq!(LegacyImuRate::new(120).unwrap().hz(), 120);
}

fn temp_sysfs_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("sim-viture-ffi-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}
