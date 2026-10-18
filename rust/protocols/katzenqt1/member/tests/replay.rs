//! katzenqt's traces, replayed through the member: the same results and the same states.

use std::path::Path;

fn testdata(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../testdata/katzenqt1")
        .join(rel)
}

fn replay_dir(dir: &Path) -> (usize, usize, usize) {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    files.sort();
    let (mut calls, mut checks) = (0, 0);
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        match pigeonhole_katzenqt1_member::trace::replay(&text) {
            Ok((n, c)) => {
                calls += n;
                checks += c;
            }
            Err(e) => panic!("{}: {e}", f.display()),
        }
    }
    (files.len(), calls, checks)
}

#[test]
fn katzenqt_traces() {
    let (files, calls, checks) = replay_dir(&testdata("groupchat_traces"));
    assert_eq!(files, 15);
    println!("{files} traces, {calls} calls, {checks} BACAP checks");
}

/// The traces in $TRACES, such as fresh random runs from katzenqt's tools/groupchat_traces.py.
#[test]
fn more_traces() {
    let Ok(dir) = std::env::var("TRACES") else {
        return;
    };
    let (files, calls, checks) = replay_dir(Path::new(&dir));
    assert!(files > 0);
    println!("{files} traces, {calls} calls, {checks} BACAP checks");
}
