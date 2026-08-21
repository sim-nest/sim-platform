#[cfg(any(feature = "music", feature = "sound"))]
use super::support::*;

#[cfg(feature = "music")]
#[test]
fn fs_dir_reads_music_artifacts_without_eval_capabilities() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[table_fs_capability(), table_fs_read_capability()],
    );
    let root = test_root("music-artifact");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("score.music"),
        "#(Score tempo=120 time_sig=4/4 key=\"C major\" body=#(Melody items=[#(Note dur=1/4 pitch=C4 vel=100 channel=0 articulation=Normal)]))",
    )
    .unwrap();

    let dir = install_fs_dir_lib(&mut cx, root.to_str().unwrap()).unwrap();
    let value = dir
        .object()
        .as_table_impl()
        .unwrap()
        .get(&mut cx, Symbol::new("score"))
        .unwrap();
    let expr = value.object().as_expr(&mut cx).unwrap();
    assert_eq!(
        expr,
        Expr::Extension {
            tag: Symbol::qualified("music", "Score"),
            payload: Box::new(Expr::String(
                "#(Score tempo=120 time_sig=4/4 key=\"C major\" body=#(Melody items=[#(Note dur=1/4 pitch=C4 vel=100 channel=0 articulation=Normal)]))".to_owned()
            )),
        }
    );
}

#[cfg(feature = "sound")]
#[test]
fn fs_dir_reads_and_rewrites_tone_artifacts() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[
            table_fs_capability(),
            table_fs_read_capability(),
            table_fs_write_capability(),
        ],
    );
    let root = test_root("tone-artifact");
    std::fs::create_dir_all(&root).unwrap();
    let canonical = "#(Tone partials=[#(Partial frequency=#(Frequency hz=440) amplitude=#(Amplitude linear=1) phase=#(Phase radians=0) tag=#(PartialTag kind=source))] envelope=#(Envelope attack=0.015 decay=0.06 sustain=0.75 release=0.12 shape=#(EnvelopeShape kind=Linear)) duration=1)";
    std::fs::write(root.join("tone.tone"), canonical).unwrap();

    let dir = install_fs_dir_lib(&mut cx, root.to_str().unwrap()).unwrap();
    let table = dir.object().as_table_impl().unwrap();
    let value = table.get(&mut cx, Symbol::new("tone")).unwrap();
    table.set(&mut cx, Symbol::new("tone"), value).unwrap();

    let rewritten = std::fs::read_to_string(root.join("tone.tone")).unwrap();
    assert_eq!(rewritten, canonical);
}
