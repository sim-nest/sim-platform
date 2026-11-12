// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use std::os::unix::fs::PermissionsExt;

fn unique_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sim-platform-{name}-{}-{}",
        process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn digest_parser_requires_canonical_sha256() {
    let digest = parse_digest(&"01".repeat(32)).unwrap();
    assert_eq!(digest[0], 1);
    assert_eq!(encode_digest(digest), "01".repeat(32));
    assert!(parse_digest(&"AB".repeat(32)).is_err());
    assert!(parse_digest("00").is_err());
    assert!(parse_digest(&"gg".repeat(32)).is_err());
}

#[test]
fn repeated_maps_refuse_duplicate_and_unbounded_limits() {
    let mut duplicate = Flags(BTreeMap::from([(
        "--build-source".into(),
        vec!["one=/a".into(), "one=/b".into()],
    )]));
    assert!(duplicate.path_map("--build-source", 14).is_err());

    let mut unbounded = Flags(BTreeMap::from([(
        "--build-limit".into(),
        vec!["one=8193:1".into()],
    )]));
    assert!(unbounded.limit_map("--build-limit", 14).is_err());
}

#[test]
fn parser_refuses_unknown_and_duplicate_scalar_inputs() {
    assert!(
        Flags::parse(["emit".into(), "--not-an-owner-input".into(), "x".into()].into_iter())
            .is_err()
    );
    let mut duplicate = Flags::parse(
        [
            "emit".into(),
            "--roadmap".into(),
            "NV12".into(),
            "--roadmap".into(),
            "substituted".into(),
        ]
        .into_iter(),
    )
    .unwrap();
    assert!(duplicate.one("--roadmap").is_err());
}

#[test]
fn owner_file_refuses_substitution_and_symlink() {
    let root = unique_root("owner-file");
    fs::create_dir(&root).unwrap();
    let path = root.join("owner.sxb");
    fs::write(&path, b"owner").unwrap();
    let wrong = OwnerFile {
        path: path.clone(),
        sha256: [0; 32],
    };
    assert!(read_owner_file(&wrong, 16).is_err());

    let digest: [u8; 32] = Sha256::digest(b"owner").into();
    let selected = OwnerFile {
        path: path.clone(),
        sha256: digest,
    };
    assert_eq!(read_owner_file(&selected, 16).unwrap(), b"owner");
    std::os::unix::fs::symlink(&path, root.join("alias")).unwrap();
    let alias = OwnerFile {
        path: root.join("alias"),
        sha256: digest,
    };
    assert!(read_owner_file(&alias, 16).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_is_all_or_nothing_and_no_replace() {
    let root = unique_root("publication");
    fs::create_dir(&root).unwrap();
    let output = root.join("selected");
    let artifacts = PreparedArtifacts {
        build_job: b"build".to_vec(),
        continuation_job: b"continuation".to_vec(),
        startup: b"startup".to_vec(),
    };
    publish(&output, &artifacts).unwrap();
    for (name, expected) in [
        (OUTPUT_NAMES[0], b"build".as_slice()),
        (OUTPUT_NAMES[1], b"continuation".as_slice()),
        (OUTPUT_NAMES[2], b"startup".as_slice()),
    ] {
        let path = output.join(name);
        assert_eq!(fs::read(&path).unwrap(), expected);
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let replacement = PreparedArtifacts {
        build_job: b"replacement".to_vec(),
        continuation_job: Vec::new(),
        startup: Vec::new(),
    };
    assert_eq!(
        publish(&output, &replacement).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(output.join(OUTPUT_NAMES[0])).unwrap(), b"build");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_failure_exposes_no_partial_target() {
    let root = unique_root("missing-parent");
    let output = root.join("missing").join("selected");
    let artifacts = PreparedArtifacts {
        build_job: b"build".to_vec(),
        continuation_job: b"continuation".to_vec(),
        startup: b"startup".to_vec(),
    };
    assert!(publish(&output, &artifacts).is_err());
    assert!(!output.exists());
}

#[test]
fn malformed_typed_input_does_not_publish() {
    let root = unique_root("malformed");
    fs::create_dir(&root).unwrap();
    let path = root.join("command.sxb");
    fs::write(&path, b"not a canonical frame").unwrap();
    let file = OwnerFile {
        path,
        sha256: Sha256::digest(b"not a canonical frame").into(),
    };
    assert!(decode_command(&file).is_err());
    assert!(!root.join("selected").exists());
    fs::remove_dir_all(root).unwrap();
}
