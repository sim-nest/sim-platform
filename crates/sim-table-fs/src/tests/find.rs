use super::support::*;

#[test]
fn grep_reports_path_and_line() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[table_fs_read_capability(), table_fs_find_capability()],
    );
    let root = test_root("find-grep-line");
    let dir = FsDir::open(root.clone()).unwrap();
    write_value_with_codec(
        &mut cx,
        &root.join("note.siml"),
        Symbol::qualified("codec", "lisp"),
        "alpha\nneedle here\nomega\n",
    );

    let result = dir.find_grep(&mut cx, "needle", None, 10).unwrap();

    assert!(!result.truncated);
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].path, "note.siml");
    assert_eq!(result.matches[0].line, 2);
    assert_eq!(result.matches[0].text, "needle here");
}

#[test]
fn glob_filters_paths() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[table_fs_read_capability(), table_fs_find_capability()],
    );
    let root = test_root("find-glob");
    let dir = FsDir::open(root.clone()).unwrap();
    std::fs::create_dir_all(root.join("notes")).unwrap();
    write_value_with_codec(
        &mut cx,
        &root.join("notes/a.siml"),
        Symbol::qualified("codec", "lisp"),
        "a",
    );
    write_value_with_codec(
        &mut cx,
        &root.join("notes/b.siml"),
        Symbol::qualified("codec", "lisp"),
        "b",
    );
    write_value_with_codec(
        &mut cx,
        &root.join("other.siml"),
        Symbol::qualified("codec", "lisp"),
        "other",
    );

    let result = dir.find_glob(&mut cx, "notes/*.siml", 10).unwrap();

    assert!(!result.truncated);
    assert_eq!(
        result.paths,
        vec!["notes/a.siml".to_owned(), "notes/b.siml".to_owned()]
    );
}

#[cfg(unix)]
#[test]
fn symlink_escape_refused() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[table_fs_read_capability(), table_fs_find_capability()],
    );
    let root = test_root("find-symlink-escape");
    let outside = test_root("find-symlink-outside").with_extension("siml");
    std::fs::write(&outside, "\"outside\"").unwrap();
    let dir = FsDir::open(root.clone()).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape.siml")).unwrap();

    let err = dir.find_glob(&mut cx, "*.siml", 10).unwrap_err();

    assert!(err.to_string().contains("escapes root"));
}

#[test]
fn max_truncation_reported() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[table_fs_read_capability(), table_fs_find_capability()],
    );
    let root = test_root("find-truncated");
    let dir = FsDir::open(root.clone()).unwrap();
    for name in ["a", "b"] {
        write_value_with_codec(
            &mut cx,
            &root.join(format!("{name}.siml")),
            Symbol::qualified("codec", "lisp"),
            "needle\n",
        );
    }

    let result = dir.find_grep(&mut cx, "needle", None, 1).unwrap();

    assert_eq!(result.matches.len(), 1);
    assert!(result.truncated);
}

#[test]
fn find_requires_find_capability() {
    let mut cx = cx();
    grant(&mut cx, &[table_fs_read_capability()]);
    let root = test_root("find-capability");
    let dir = FsDir::open(root.clone()).unwrap();
    write_value_with_codec(
        &mut cx,
        &root.join("note.siml"),
        Symbol::qualified("codec", "lisp"),
        "needle",
    );

    let err = dir.find_grep(&mut cx, "needle", None, 10).unwrap_err();

    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == table_fs_find_capability()
    ));
}
