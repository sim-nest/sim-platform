use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use sim_codec::encode_with_codec;
use sim_codec_algol::AlgolCodecLib;
use sim_codec_binary::BinaryCodecLib;
use sim_codec_binary_base64::BinaryBase64CodecLib;
use sim_codec_json::JsonCodecLib;
use sim_codec_lisp::LispCodecLib;
use sim_kernel::{DefaultFactory, EagerPolicy, EncodeOptions};

pub(super) use crate::{
    FsDir, FsDirDescriptor, fs_dir_class_symbol, install_fs_dir_lib, table_fs_capability,
    table_fs_edit_capability, table_fs_find_capability, table_fs_mkdir_capability,
    table_fs_read_capability, table_fs_rmdir_capability, table_fs_write_capability,
};
pub(super) use sim_kernel::{Dir, Expr, ObjectEncoding, Symbol, Table, read_construct_capability};

pub(super) fn test_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sim-table-fs-{name}-{}-{nanos}",
        std::process::id()
    ))
}

pub(super) fn cx() -> sim_kernel::Cx {
    let mut cx = sim_kernel::Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    sim_test_support::register_core_classes(&mut cx);
    let lisp_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&LispCodecLib::new(lisp_id).unwrap()).unwrap();
    let json_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&JsonCodecLib::new(json_id)).unwrap();
    let binary_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&BinaryCodecLib::new(binary_id)).unwrap();
    let binary_base64_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&BinaryBase64CodecLib::new(binary_base64_id))
        .unwrap();
    let algol_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&AlgolCodecLib::new(algol_id)).unwrap();
    cx
}

pub(super) fn grant(cx: &mut sim_kernel::Cx, capabilities: &[sim_kernel::CapabilityName]) {
    for capability in capabilities {
        cx.grant(capability.clone());
    }
}

pub(super) fn grant_edit_authority(cx: &mut sim_kernel::Cx) {
    grant(
        cx,
        &[
            table_fs_read_capability(),
            table_fs_write_capability(),
            table_fs_edit_capability(),
        ],
    );
}

pub(super) fn write_value_with_codec(
    cx: &mut sim_kernel::Cx,
    path: &PathBuf,
    codec: Symbol,
    value: &str,
) {
    let expr = Expr::String(value.to_owned());
    let output = encode_with_codec(cx, &codec, &expr, EncodeOptions::default()).unwrap();
    let bytes = match output {
        sim_codec::Output::Text(text) => text.into_bytes(),
        sim_codec::Output::Bytes(bytes) => bytes,
    };
    std::fs::write(path, bytes).unwrap();
}
