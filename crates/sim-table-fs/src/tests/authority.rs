use super::support::*;

#[test]
fn del_requires_read_before_returning_removed_value() {
    let mut cx = cx();
    grant(&mut cx, &[table_fs_write_capability()]);
    let root = test_root("del-write-only");
    let dir = FsDir::open(root.clone()).unwrap();
    std::fs::write(root.join("note.siml"), "\"secret\"").unwrap();

    let err = dir.del(&mut cx, Symbol::new("note")).unwrap_err();

    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == table_fs_read_capability()
    ));
    assert!(root.join("note.siml").exists());
}

#[test]
fn del_decode_error_leaves_file_in_place() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[table_fs_read_capability(), table_fs_write_capability()],
    );
    let root = test_root("del-bad-codec");
    let dir = FsDir::open(root.clone()).unwrap();
    std::fs::write(root.join("bad.siml"), "(").unwrap();

    dir.del(&mut cx, Symbol::new("bad")).unwrap_err();

    assert!(root.join("bad.siml").exists());
}

#[test]
fn dir_edit_round_trips_with_fs_write() {
    let mut cx = cx();
    grant_edit_authority(&mut cx);
    let root = test_root("dir-edit-roundtrip");
    let dir = FsDir::open(root.clone()).unwrap();
    write_value_with_codec(
        &mut cx,
        &root.join("note.siml"),
        Symbol::qualified("codec", "lisp"),
        "alpha beta",
    );

    dir.edit(&mut cx, Symbol::new("note"), "beta", "gamma", false)
        .unwrap();

    let value = dir.get(&mut cx, Symbol::new("note")).unwrap();
    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("alpha gamma".to_owned())
    );
}

#[test]
fn dir_edit_lines_round_trips_with_fs_write() {
    let mut cx = cx();
    grant_edit_authority(&mut cx);
    let root = test_root("dir-edit-lines");
    let dir = FsDir::open(root.clone()).unwrap();
    write_value_with_codec(
        &mut cx,
        &root.join("note.siml"),
        Symbol::qualified("codec", "lisp"),
        "a\nb\nc\n",
    );

    dir.edit_lines(&mut cx, Symbol::new("note"), 2, 2, "B\n")
        .unwrap();

    let value = dir.get(&mut cx, Symbol::new("note")).unwrap();
    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("a\nB\nc\n".to_owned())
    );
}

#[test]
fn dir_edit_is_atomic_on_failure() {
    let mut cx = cx();
    grant_edit_authority(&mut cx);
    let root = test_root("dir-edit-atomic");
    let dir = FsDir::open(root.clone()).unwrap();
    let path = root.join("note.siml");
    write_value_with_codec(
        &mut cx,
        &path,
        Symbol::qualified("codec", "lisp"),
        "alpha beta",
    );
    let before = std::fs::read_to_string(&path).unwrap();

    let err = dir
        .edit(&mut cx, Symbol::new("note"), "missing", "gamma", false)
        .unwrap_err();

    assert!(err.to_string().contains("pattern not found"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
}

#[test]
fn dir_edit_requires_edit_capability() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[table_fs_read_capability(), table_fs_write_capability()],
    );
    let root = test_root("dir-edit-cap");
    let dir = FsDir::open(root.clone()).unwrap();
    let path = root.join("note.siml");
    write_value_with_codec(
        &mut cx,
        &path,
        Symbol::qualified("codec", "lisp"),
        "alpha beta",
    );

    let err = dir
        .edit(&mut cx, Symbol::new("note"), "beta", "gamma", false)
        .unwrap_err();

    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == table_fs_edit_capability()
    ));
    let value = dir.get(&mut cx, Symbol::new("note")).unwrap();
    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("alpha beta".to_owned())
    );
}

#[test]
fn dir_edit_requires_write_capability() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[table_fs_read_capability(), table_fs_edit_capability()],
    );
    let root = test_root("dir-edit-write-cap");
    let dir = FsDir::open(root.clone()).unwrap();
    write_value_with_codec(
        &mut cx,
        &root.join("note.siml"),
        Symbol::qualified("codec", "lisp"),
        "alpha beta",
    );

    let err = dir
        .edit(&mut cx, Symbol::new("note"), "beta", "gamma", false)
        .unwrap_err();

    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == table_fs_write_capability()
    ));
}
