use super::support::*;

// conformance: host storage primitives read and write Table rows.

#[test]
fn fs_dir_set_get_roundtrip_and_extension_selection() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[
            table_fs_capability(),
            table_fs_read_capability(),
            table_fs_write_capability(),
        ],
    );

    let root = test_root("roundtrip");
    let dir = install_fs_dir_lib(&mut cx, root.to_str().unwrap()).unwrap();
    let table = dir.object().as_table_impl().unwrap();
    let hello = cx.factory().string("hello".to_owned()).unwrap();

    table.set(&mut cx, Symbol::new("alpha"), hello).unwrap();
    assert!(root.join("alpha.siml").is_file());
    let alpha = table.get(&mut cx, Symbol::new("alpha")).unwrap();
    assert_eq!(
        alpha.object().as_expr(&mut cx).unwrap(),
        Expr::String("hello".to_owned())
    );

    write_value_with_codec(
        &mut cx,
        &root.join("json-value.simj"),
        Symbol::qualified("codec", "json"),
        "json",
    );
    write_value_with_codec(
        &mut cx,
        &root.join("binary-value.simb"),
        Symbol::qualified("codec", "binary"),
        "binary",
    );
    write_value_with_codec(
        &mut cx,
        &root.join("binary-base64-value.simb64"),
        Symbol::qualified("codec", "binary-base64"),
        "binary-base64",
    );
    write_value_with_codec(
        &mut cx,
        &root.join("algol-value.sima"),
        Symbol::qualified("codec", "algol"),
        "algol",
    );

    for (key, expected) in [
        ("json-value", "json"),
        ("binary-value", "binary"),
        ("binary-base64-value", "binary-base64"),
        ("algol-value", "algol"),
    ] {
        let value = table.get(&mut cx, Symbol::new(key)).unwrap();
        assert_eq!(
            value.object().as_expr(&mut cx).unwrap(),
            Expr::String(expected.to_owned())
        );
    }
}

#[test]
fn fs_dir_mkdir_opendir_rmdir_and_path_guards() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[
            table_fs_capability(),
            table_fs_read_capability(),
            table_fs_mkdir_capability(),
            table_fs_rmdir_capability(),
        ],
    );

    let root = test_root("dirs");
    let dir = FsDir::open(root.clone()).unwrap();
    let sub = dir.mkdir(&mut cx, Symbol::new("sub")).unwrap();
    assert!(sub.object().as_dir().is_some());
    assert!(dir.is_dir(&mut cx, Symbol::new("sub")).unwrap());
    assert!(dir.opendir(&mut cx, Symbol::new("sub")).unwrap().is_some());
    dir.rmdir(&mut cx, Symbol::new("sub")).unwrap();
    assert!(!root.join("sub").exists());

    for illegal in ["/tmp", "..", ".", "a/b", "a\\b"] {
        let err = dir.mkdir(&mut cx, Symbol::new(illegal)).unwrap_err();
        assert!(err.to_string().contains("illegal name"));
    }
}

#[test]
fn fs_dir_operations_are_capability_gated() {
    let mut cx = cx();
    grant(&mut cx, &[table_fs_capability()]);

    let root = test_root("caps");
    let dir = install_fs_dir_lib(&mut cx, root.to_str().unwrap()).unwrap();
    let table = dir.object().as_table_impl().unwrap();
    let fs_dir = dir.object().as_dir().unwrap();
    let value = cx.factory().string("value".to_owned()).unwrap();

    assert!(matches!(
        table.get(&mut cx, Symbol::new("x")),
        Err(sim_kernel::Error::CapabilityDenied { .. })
    ));
    assert!(matches!(
        table.set(&mut cx, Symbol::new("x"), value),
        Err(sim_kernel::Error::CapabilityDenied { .. })
    ));
    assert!(matches!(
        fs_dir.mkdir(&mut cx, Symbol::new("sub")),
        Err(sim_kernel::Error::CapabilityDenied { .. })
    ));
    assert!(matches!(
        fs_dir.rmdir(&mut cx, Symbol::new("sub")),
        Err(sim_kernel::Error::CapabilityDenied { .. })
    ));
}

#[test]
fn fs_dir_accepts_compatibility_capability_aliases() {
    let root = test_root("compat-caps");
    let dir = FsDir::open(root).unwrap();

    let mut rw_cx = cx();
    grant(
        &mut rw_cx,
        &[
            sim_kernel::CapabilityName::new("table.fs.read"),
            sim_kernel::CapabilityName::new("table.fs.write"),
        ],
    );
    let value = rw_cx.factory().string("value".to_owned()).unwrap();
    dir.set(&mut rw_cx, Symbol::new("x"), value).unwrap();
    assert_eq!(
        dir.get(&mut rw_cx, Symbol::new("x"))
            .unwrap()
            .object()
            .as_expr(&mut rw_cx)
            .unwrap(),
        Expr::String("value".to_owned())
    );

    let mut mkdir_cx = cx();
    grant(
        &mut mkdir_cx,
        &[sim_kernel::CapabilityName::new("table.fs.mkdir")],
    );
    dir.mkdir(&mut mkdir_cx, Symbol::new("sub")).unwrap();

    let mut rmdir_cx = cx();
    grant(
        &mut rmdir_cx,
        &[sim_kernel::CapabilityName::new("table.fs.rmdir")],
    );
    dir.rmdir(&mut rmdir_cx, Symbol::new("sub")).unwrap();
}

#[test]
fn fs_dir_rejects_traversal_inputs_for_get_set_and_mkdir() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[
            table_fs_capability(),
            table_fs_read_capability(),
            table_fs_write_capability(),
            table_fs_mkdir_capability(),
        ],
    );

    let root = test_root("traversal");
    let dir = install_fs_dir_lib(&mut cx, root.to_str().unwrap()).unwrap();
    let table = dir.object().as_table_impl().unwrap();
    let fs_dir = dir.object().as_dir().unwrap();
    let value = cx.factory().string("value".to_owned()).unwrap();

    for illegal in ["..", "/etc/passwd", "a/b", "a\\b"] {
        let key = Symbol::new(illegal);

        let err = table.get(&mut cx, key.clone()).unwrap_err();
        assert!(err.to_string().contains("illegal name") || err.to_string().contains("escapes"));

        let err = table.set(&mut cx, key.clone(), value.clone()).unwrap_err();
        assert!(err.to_string().contains("illegal name") || err.to_string().contains("escapes"));

        let err = fs_dir.mkdir(&mut cx, key).unwrap_err();
        assert!(err.to_string().contains("illegal name") || err.to_string().contains("escapes"));
    }

    assert!(!root.join("a").exists());
    assert!(!root.join("etc").exists());
}

#[test]
fn fs_dir_rejects_illegal_segments_via_shared_predicate() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[
            table_fs_capability(),
            table_fs_read_capability(),
            table_fs_write_capability(),
            table_fs_mkdir_capability(),
        ],
    );

    let root = test_root("illegal-segments");
    let dir = install_fs_dir_lib(&mut cx, root.to_str().unwrap()).unwrap();
    let table = dir.object().as_table_impl().unwrap();
    let fs_dir = dir.object().as_dir().unwrap();

    // Names that `is_legal_table_segment` rejects, plus the table-fs-only
    // absolute-path guard, must all fail closed with the table-fs message.
    for illegal in ["", ".", "..", "a/b", "a\\b", "/abs"] {
        assert!(!sim_table_core::is_legal_table_segment(illegal) || illegal == "/abs");
        let key = Symbol::new(illegal);
        let err = table.get(&mut cx, key.clone()).unwrap_err();
        assert!(err.to_string().contains("illegal name") || err.to_string().contains("escapes"));
        let err = fs_dir.mkdir(&mut cx, key).unwrap_err();
        assert!(err.to_string().contains("illegal name") || err.to_string().contains("escapes"));
    }
}
