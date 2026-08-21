use std::sync::Arc;

use super::support::*;

#[test]
fn fs_dir_citizen_round_trips_as_descriptor_only() {
    let mut cx = cx();
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    cx.grant(read_construct_capability());
    let root = test_root("citizen");
    let dir = FsDir::open(root).unwrap();
    let original = cx.factory().opaque(Arc::new(dir)).unwrap();

    sim_citizen::check_value_fixture(&mut cx, original.clone()).unwrap();

    let ObjectEncoding::Constructor { args, .. } = original
        .object()
        .as_object_encoder()
        .unwrap()
        .object_encoding(&mut cx)
        .unwrap()
    else {
        panic!("expected constructor encoding");
    };
    let args = args
        .iter()
        .map(|arg| sim_citizen::value_from_expr(&mut cx, arg))
        .collect::<sim_kernel::Result<Vec<_>>>()
        .unwrap();
    let decoded = cx.read_construct(&fs_dir_class_symbol(), args).unwrap();

    assert!(
        decoded
            .object()
            .as_any()
            .downcast_ref::<FsDirDescriptor>()
            .is_some()
    );
    assert!(decoded.object().as_table_impl().is_none());
    assert!(decoded.object().as_dir().is_none());
}
