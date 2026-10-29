//! A store sealed with a key: nothing it keeps is readable without that key.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use pigeonhole_protocols::protocols::katzenqt1::client::Files;
use pigeonhole_protocols::protocols::katzenqt1::client::store::{Options, Store, StoreError};
use pigeonhole_protocols::protocols::katzenqt1::engine::{
    Changes, Chat, FileWrite, Line, State, StepResult,
};
use pigeonhole_protocols::secret::Secret;
use redb::ReadableTable;

const WORDS: &[u8] = b"meet me by the old mill";
const FILE: &[u8] = b"the contents of an attachment";

fn tmp() -> tempfile::TempDir {
    tempfile::TempDir::new_in(
        std::env::var("TMPDIR")
            .unwrap_or_else(|_| format!("{}/scratch/tmp", std::env::var("HOME").unwrap())),
    )
    .unwrap()
}

fn key(b: u8) -> Option<Secret> {
    Some(Secret::new(&[b; 32]))
}

fn opts(key: Option<Secret>) -> Options {
    Options { no_sync: true, key }
}

fn chat(id: &str) -> Chat {
    Chat {
        id: id.into(),
        name: format!("{id} group"),
        write_cap: Secret::new(b"a write cap of the group"),
        log: vec![Line {
            id: 0,
            text: String::from_utf8(WORDS.to_vec()).unwrap(),
            ..Line::default()
        }],
        ..Chat::default()
    }
}

/// Saves chats `ids`, each with one line, and a file.
fn fill(st: &mut Store, ids: &[&str]) -> State {
    let state = State {
        me: "alice".into(),
        chats: ids.iter().map(|i| chat(i)).collect(),
    };
    let r = StepResult {
        files: vec![FileWrite {
            file: "attachments/g/a.bin".into(),
            bytes: FILE.to_vec(),
        }],
        changes: Changes {
            chats: ids.iter().map(|i| i.to_string()).collect(),
            lines: ids
                .iter()
                .map(|i| (i.to_string(), BTreeSet::from([0])))
                .collect::<BTreeMap<_, _>>(),
            ..Changes::default()
        },
        ..StepResult::default()
    };
    st.commit(&state, &r).unwrap();
    state
}

fn holds(dir: &Path, needle: &[u8]) -> bool {
    walk(dir).iter().any(|f| {
        std::fs::read(f)
            .unwrap()
            .windows(needle.len())
            .any(|w| w == needle)
    })
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

fn locked<T>(r: Result<T, StoreError>) -> bool {
    matches!(r, Err(StoreError::Locked))
}

#[test]
fn sealed_round_trip() {
    let dir = tmp();
    {
        let mut st = Store::open(dir.path(), opts(key(1))).unwrap();
        fill(&mut st, &["g"]);
    }
    assert!(!holds(dir.path(), WORDS));
    assert!(!holds(dir.path(), FILE));
    assert!(!holds(dir.path(), b"alice"));
    assert!(!holds(dir.path(), b"a write cap of the group"));
    let mut st = Store::open(dir.path(), opts(key(1))).unwrap();
    let state = st.load("bob").unwrap();
    assert_eq!(state.me, "alice");
    assert_eq!(state.chats[0].name, "g group");
    assert_eq!(state.chats[0].log[0].text.as_bytes(), WORDS);
    assert_eq!(&*state.chats[0].write_cap, b"a write cap of the group");
    assert_eq!(st.files().read("attachments/g/a.bin").unwrap(), FILE);
    assert!(st.unsaved(&state).unwrap().is_empty());
}

#[test]
fn wrong_or_no_key_is_locked() {
    let dir = tmp();
    {
        let mut st = Store::open(dir.path(), opts(key(1))).unwrap();
        fill(&mut st, &["g"]);
    }
    assert!(locked(Store::open(dir.path(), opts(key(2)))));
    assert!(locked(Store::open(dir.path(), opts(None))));
}

#[test]
fn plain_opened_with_a_key_is_unsealed() {
    let dir = tmp();
    {
        let mut st = Store::open(dir.path(), opts(None)).unwrap();
        fill(&mut st, &["g"]);
    }
    assert!(matches!(
        Store::open(dir.path(), opts(key(1))),
        Err(StoreError::Unsealed)
    ));
}

#[test]
fn a_fresh_store_with_a_key_is_sealed() {
    let dir = tmp();
    drop(Store::open(dir.path(), opts(key(1))).unwrap());
    assert!(locked(Store::open(dir.path(), opts(None))));
}

/// A value moved to another conversation's row does not open there.
#[test]
fn moved_values_do_not_open() {
    let dir = tmp();
    {
        let mut st = Store::open(dir.path(), opts(key(1))).unwrap();
        fill(&mut st, &["g", "h"]);
    }
    {
        let db = redb::Database::open(dir.path().join("state.redb")).unwrap();
        let t: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("records");
        let tx = db.begin_write().unwrap();
        {
            let mut rt = tx.open_table(t).unwrap();
            let g = rt.get("g").unwrap().unwrap().value().to_vec();
            rt.insert("h", g.as_slice()).unwrap();
        }
        tx.commit().unwrap();
    }
    let mut st = Store::open(dir.path(), opts(key(1))).unwrap();
    assert!(locked(st.load("alice")));
}

#[test]
fn a_moved_file_does_not_open() {
    let dir = tmp();
    {
        let mut st = Store::open(dir.path(), opts(key(1))).unwrap();
        fill(&mut st, &["g"]);
    }
    let from = dir.path().join("attachments/g/a.bin");
    std::fs::copy(&from, dir.path().join("attachments/g/b.bin")).unwrap();
    let st = Store::open(dir.path(), opts(key(1))).unwrap();
    assert!(st.files().read("attachments/g/b.bin").is_err());
}

#[test]
fn reseal_keeps_everything() {
    let (from, to) = (tmp(), tmp());
    let to = to.path().join("state");
    let want = {
        let mut st = Store::open(from.path(), opts(None)).unwrap();
        fill(&mut st, &["g", "h"])
    };
    Store::reseal(from.path(), &to, Secret::new(&[1; 32])).unwrap();
    assert!(!holds(&to, WORDS));
    assert!(!holds(&to, FILE));
    let mut st = Store::open(&to, opts(key(1))).unwrap();
    let got = st.load("bob").unwrap();
    assert_eq!(got.me, want.me);
    let names = |s: &State| s.chats.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&got), names(&want));
    assert_eq!(got.chats[1].log, want.chats[1].log);
    assert_eq!(st.files().read("attachments/g/a.bin").unwrap(), FILE);
    assert!(st.unsaved(&got).unwrap().is_empty());
}
