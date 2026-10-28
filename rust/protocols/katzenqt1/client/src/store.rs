//! A katzenqt1 client's state on disk: one redb file, with a record per conversation and its
//! lines kept one by one, and the files lines and uploads carry beside it. A step's changes are
//! saved in one transaction, synced to disk before [`Store::commit`] returns.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

use pigeonhole_katzenqt1_engine::*;
use pigeonhole_katzenqt1_member::trace;
use pigeonhole_transport::Secret;
use redb::{Database, Durability, ReadableTable, TableDefinition};
use serde_json::{Value, json};

use crate::exec::Files;

const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const RECORDS: TableDefinition<&str, &[u8]> = TableDefinition::new("records");
const LINES: TableDefinition<(&str, u64), &[u8]> = TableDefinition::new("lines");
const VERSION: &[u8] = &[1];

#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "store: {}", self.0)
    }
}

impl std::error::Error for StoreError {}

fn e(x: impl std::fmt::Display) -> StoreError {
    StoreError(x.to_string())
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Leave syncing to the system: for tests only, since a crash may then lose what was
    /// committed.
    pub no_sync: bool,
}

pub struct Store {
    dir: PathBuf,
    db: Database,
    opts: Options,
    /// Each conversation's record as last saved, so that one unchanged is not written.
    saved: HashMap<ChatId, Vec<u8>>,
}

fn h(b: &[u8]) -> Value {
    Value::String(hex::encode(b))
}

fn oh<B: AsRef<[u8]>>(b: &Option<B>) -> Value {
    b.as_ref().map_or(Value::Null, |b| h(b.as_ref()))
}

fn chat_json(c: &Chat) -> Value {
    json!({
        "id": c.id, "name": c.name, "write_cap": h(&c.write_cap), "next_write": h(&c.next_write),
        "group": c.group.as_ref().map(trace::group),
        "cursors": c.cursors.iter().map(|(k, v)| json!([h(k), h(v)])).collect::<Vec<_>>(),
        "names": c.names.iter().map(|(k, v)| json!([h(k), v])).collect::<Vec<_>>(),
        "jobs": c.jobs.iter().map(|j| json!({
            "id": j.id, "token": j.token, "index": oh(&j.index), "box": h(&j.box_), "line": j.line,
            "upload": j.upload.as_ref().map(|u| json!({"file": u.file, "box_size": u.box_size, "total": u.total,
                "write_cap": h(&u.write_cap), "sent": u.sent, "next": h(&u.next)})),
        })).collect::<Vec<_>>(),
        "joining": c.joining.as_ref().map(|j| json!({"code": j.code, "secret_key": h(&j.secret_key),
            "voucher_read_cap": h(&j.voucher_read_cap), "box1": h(&j.box1)})),
        "inducting": c.inducting.as_ref().map(|i| json!({"voucher": h(&i.voucher), "payload": oh(&i.payload),
            "voucher_write_cap": h(&i.voucher_write_cap), "box1": h(&i.box1), "sealing": i.sealing,
            "sealed": i.sealed.as_ref().map(|s| json!({"name": s.name, "mutated_read_cap": h(&s.mutated_read_cap), "reply": h(&s.reply)}))})),
        "next_id": c.next_id,
        "downloads": c.downloads.iter().map(|d| json!({"stream": h(&d.stream), "index": h(&d.index), "next": h(&d.next),
            "read_cap": h(&d.read_cap), "total": d.total})).collect::<Vec<_>>(),
    })
}

fn line_json(l: &Line) -> Value {
    json!({
        "id": l.id, "sender": h(&l.sender), "index": h(&l.index), "text": l.text,
        "notice": l.notice.as_ref().map(|n| json!({"kind": n.kind.code(), "args": n.args})),
        "status": matches!(l.status, Status::Pending), "idx64": l.idx64, "time": l.time,
        "attachment": l.attachment.as_ref().map(|a| json!({"name": a.name, "filetype": a.filetype, "size": a.size, "file": a.file})),
    })
}

fn bytes(j: &Value) -> Result<Vec<u8>> {
    hex::decode(j.as_str().ok_or_else(|| e("not a string"))?).map_err(e)
}

fn secret(j: &Value) -> Result<Secret> {
    bytes(j).map(Secret::from)
}

fn opt_secret(j: &Value) -> Result<Option<Secret>> {
    Ok(opt_bytes(j)?.map(Secret::from))
}

fn opt_bytes(j: &Value) -> Result<Option<Vec<u8>>> {
    if j.is_null() {
        Ok(None)
    } else {
        bytes(j).map(Some)
    }
}

fn num(j: &Value) -> Result<usize> {
    j.as_u64()
        .map(|n| n as usize)
        .ok_or_else(|| e(format!("not a number: {j}")))
}

fn opt_num(j: &Value) -> Result<Option<usize>> {
    if j.is_null() {
        Ok(None)
    } else {
        num(j).map(Some)
    }
}

fn text(j: &Value) -> Result<String> {
    j.as_str()
        .map(String::from)
        .ok_or_else(|| e("not a string"))
}

fn list(j: &Value) -> Result<&Vec<Value>> {
    j.as_array().ok_or_else(|| e("not a list"))
}

fn chat_of(j: &Value) -> Result<Chat> {
    let mut c = Chat {
        id: text(&j["id"])?,
        name: text(&j["name"])?,
        write_cap: secret(&j["write_cap"])?,
        next_write: bytes(&j["next_write"])?,
        next_id: num(&j["next_id"])?,
        ..Chat::default()
    };
    if !j["group"].is_null() {
        c.group = Some(trace::read_group(&j["group"]).map_err(e)?);
    }
    for p in list(&j["cursors"])? {
        c.cursors.insert(bytes(&p[0])?, bytes(&p[1])?);
    }
    for p in list(&j["names"])? {
        c.names.insert(bytes(&p[0])?, text(&p[1])?);
    }
    for x in list(&j["jobs"])? {
        let u = &x["upload"];
        c.jobs.push(Job {
            id: num(&x["id"])?,
            token: opt_num(&x["token"])?,
            index: opt_bytes(&x["index"])?,
            box_: bytes(&x["box"])?,
            line: opt_num(&x["line"])?,
            upload: if u.is_null() {
                None
            } else {
                Some(Upload {
                    file: text(&u["file"])?,
                    box_size: num(&u["box_size"])?,
                    total: num(&u["total"])?,
                    write_cap: secret(&u["write_cap"])?,
                    sent: num(&u["sent"])?,
                    next: bytes(&u["next"])?,
                })
            },
        });
    }
    let jn = &j["joining"];
    if !jn.is_null() {
        c.joining = Some(Joining {
            code: text(&jn["code"])?,
            secret_key: secret(&jn["secret_key"])?,
            voucher_read_cap: secret(&jn["voucher_read_cap"])?,
            box1: bytes(&jn["box1"])?,
        });
    }
    let i = &j["inducting"];
    if !i.is_null() {
        let s = &i["sealed"];
        c.inducting = Some(Inducting {
            voucher: secret(&i["voucher"])?,
            payload: opt_secret(&i["payload"])?,
            voucher_write_cap: secret(&i["voucher_write_cap"])?,
            box1: bytes(&i["box1"])?,
            sealing: i["sealing"].as_bool().unwrap_or(false),
            sealed: if s.is_null() {
                None
            } else {
                Some(Sealed {
                    name: text(&s["name"])?,
                    mutated_read_cap: secret(&s["mutated_read_cap"])?,
                    reply: bytes(&s["reply"])?,
                })
            },
        });
    }
    for d in list(&j["downloads"])? {
        c.downloads.push(Download {
            stream: bytes(&d["stream"])?,
            index: bytes(&d["index"])?,
            next: bytes(&d["next"])?,
            read_cap: secret(&d["read_cap"])?,
            total: opt_num(&d["total"])?,
        });
    }
    Ok(c)
}

fn line_of(j: &Value) -> Result<Line> {
    let n = &j["notice"];
    let a = &j["attachment"];
    Ok(Line {
        id: num(&j["id"])?,
        sender: bytes(&j["sender"])?,
        index: bytes(&j["index"])?,
        text: text(&j["text"])?,
        notice: if n.is_null() {
            None
        } else {
            let kind = NoticeKind::from_code(num(&n["kind"])? as u32)
                .ok_or_else(|| e("unknown notice"))?;
            Some(Notice {
                kind,
                args: list(&n["args"])?.iter().map(text).collect::<Result<_>>()?,
            })
        },
        status: if j["status"].as_bool() == Some(true) {
            Status::Pending
        } else {
            Status::Sent
        },
        idx64: j["idx64"].as_u64(),
        time: j["time"].as_i64().unwrap_or(0),
        attachment: if a.is_null() {
            None
        } else {
            Some(Attachment {
                name: text(&a["name"])?,
                filetype: text(&a["filetype"])?,
                size: num(&a["size"])?,
                file: text(&a["file"])?,
            })
        },
    })
}

fn encode(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).expect("JSON is written")
}

fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

impl Store {
    /// Open the state in `dir`, making it if need be.
    pub fn open(dir: impl AsRef<Path>, opts: Options) -> Result<Store> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir).map_err(e)?;
        let db = Database::create(dir.join("state.redb")).map_err(e)?;
        let tx = db.begin_write().map_err(e)?;
        {
            let mut m = tx.open_table(META).map_err(e)?;
            let v = m.get("version").map_err(e)?.map(|v| v.value().to_vec());
            match v {
                None => {
                    m.insert("version", VERSION).map_err(e)?;
                }
                Some(v) if v == VERSION => {}
                Some(v) => return Err(e(format!("state of version {v:?}"))),
            }
            tx.open_table(RECORDS).map_err(e)?;
            tx.open_table(LINES).map_err(e)?;
        }
        tx.commit().map_err(e)?;
        Ok(Store {
            dir,
            db,
            opts,
            saved: HashMap::new(),
        })
    }

    /// The whole state; `me` is our name when none is saved.
    pub fn load(&mut self, me: &str) -> Result<State> {
        let tx = self.db.begin_read().map_err(e)?;
        let meta = tx.open_table(META).map_err(e)?;
        let records = tx.open_table(RECORDS).map_err(e)?;
        let lines = tx.open_table(LINES).map_err(e)?;
        let mut st = State {
            me: me.into(),
            chats: Vec::new(),
        };
        if let Some(n) = meta.get("me").map_err(e)? {
            st.me = String::from_utf8_lossy(n.value()).into();
        }
        let order: Vec<String> = match meta.get("order").map_err(e)? {
            Some(o) => serde_json::from_slice(o.value()).map_err(e)?,
            None => Vec::new(),
        };
        for id in order {
            let rec = records
                .get(id.as_str())
                .map_err(e)?
                .ok_or_else(|| e(format!("no conversation {id}")))?
                .value()
                .to_vec();
            let mut c = chat_of(&serde_json::from_slice(&rec).map_err(e)?)?;
            for l in lines
                .range((id.as_str(), 0)..=(id.as_str(), u64::MAX))
                .map_err(e)?
            {
                let (_, v) = l.map_err(e)?;
                c.log
                    .push(line_of(&serde_json::from_slice(v.value()).map_err(e)?)?);
            }
            self.saved.insert(id, rec);
            st.chats.push(c);
        }
        Ok(st)
    }

    /// Where a file under the state directory is, refusing names that leave it.
    pub fn path(&self, name: &str) -> Result<PathBuf> {
        within(&self.dir, name).map_err(|_| e(format!("{name:?} is outside the state directory")))
    }

    /// Write a file and sync it and its directory, so that it is on disk before any state that
    /// names it.
    fn keep(&self, f: &FileWrite) -> Result<()> {
        let p = self.path(&f.file)?;
        let dir = p.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).map_err(e)?;
        let tmp = dir.join(format!(".tmp-{}", rand::random::<u64>()));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut file| {
                file.write_all(&f.bytes)?;
                if !self.opts.no_sync {
                    file.sync_all()?;
                }
                Ok(())
            })
            .and_then(|()| std::fs::rename(&tmp, &p));
        if let Err(x) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(e(x));
        }
        if !self.opts.no_sync {
            sync_dir(&dir).map_err(e)?;
        }
        Ok(())
    }

    /// Save what a step changed: first the files it keeps, then its changes to the state, in one
    /// transaction. Nothing the step asks for may be done before this returns without error.
    pub fn commit(&mut self, st: &State, r: &StepResult) -> Result<()> {
        for f in &r.files {
            self.keep(f)?;
        }
        let ch = &r.changes;
        let mut records: Vec<(ChatId, Vec<u8>)> = Vec::new();
        for id in &ch.chats {
            let Some(c) = st.chat(id) else { continue };
            let rec = encode(&chat_json(c));
            if self.saved.get(id) != Some(&rec) {
                records.push((id.clone(), rec));
            }
        }
        if records.is_empty()
            && ch.lines.is_empty()
            && ch.dropped.is_empty()
            && ch.removed.is_empty()
        {
            return Ok(());
        }
        let mut tx = self.db.begin_write().map_err(e)?;
        if self.opts.no_sync {
            tx.set_durability(Durability::None);
        }
        {
            let mut rt = tx.open_table(RECORDS).map_err(e)?;
            let mut lt = tx.open_table(LINES).map_err(e)?;
            for id in &ch.removed {
                rt.remove(id.as_str()).map_err(e)?;
                let gone: Vec<u64> = lt
                    .range((id.as_str(), 0)..=(id.as_str(), u64::MAX))
                    .map_err(e)?
                    .map(|r| r.map(|(k, _)| k.value().1))
                    .collect::<std::result::Result<_, _>>()
                    .map_err(e)?;
                for n in gone {
                    lt.remove((id.as_str(), n)).map_err(e)?;
                }
            }
            for (id, rec) in &records {
                rt.insert(id.as_str(), rec.as_slice()).map_err(e)?;
            }
            for (id, ls) in &ch.lines {
                let Some(c) = st.chat(id) else { continue };
                for l in c.log.iter().filter(|l| ls.contains(&l.id)) {
                    lt.insert((id.as_str(), l.id as u64), encode(&line_json(l)).as_slice())
                        .map_err(e)?;
                }
            }
            for (id, ls) in &ch.dropped {
                for &n in ls {
                    lt.remove((id.as_str(), n as u64)).map_err(e)?;
                }
            }
            let mut m = tx.open_table(META).map_err(e)?;
            let order: Vec<&str> = st.chats.iter().map(|c| c.id.as_str()).collect();
            m.insert("order", serde_json::to_vec(&order).unwrap().as_slice())
                .map_err(e)?;
            m.insert("me", st.me.as_bytes()).map_err(e)?;
        }
        tx.commit().map_err(e)?;
        for (id, rec) in records {
            self.saved.insert(id, rec);
        }
        for id in &ch.removed {
            self.saved.remove(id);
        }
        Ok(())
    }

    /// The conversations whose record or lines differ from what was last saved: a step that
    /// changed them without saying so. For tests.
    pub fn unsaved(&self, st: &State) -> Result<Vec<ChatId>> {
        let tx = self.db.begin_read().map_err(e)?;
        let rt = tx.open_table(RECORDS).map_err(e)?;
        let lt = tx.open_table(LINES).map_err(e)?;
        let mut out = Vec::new();
        for c in &st.chats {
            let rec = encode(&chat_json(c));
            let saved = rt
                .get(c.id.as_str())
                .map_err(e)?
                .map(|v| v.value().to_vec());
            let lines: Vec<Vec<u8>> = lt
                .range((c.id.as_str(), 0)..=(c.id.as_str(), u64::MAX))
                .map_err(e)?
                .map(|r| r.map(|(_, v)| v.value().to_vec()))
                .collect::<std::result::Result<_, _>>()
                .map_err(e)?;
            let want: Vec<Vec<u8>> = c.log.iter().map(|l| encode(&line_json(l))).collect();
            if saved.as_ref() != Some(&rec) || lines != want {
                out.push(c.id.clone());
            }
        }
        Ok(out)
    }
}

/// The files under the state directory, for an [`crate::Executor`].
pub struct StoreFiles(pub PathBuf);

impl StoreFiles {
    fn path(&self, name: &str) -> std::io::Result<PathBuf> {
        within(&self.0, name)
    }
}

/// The file `name` names under `dir`, which it may not leave.
fn within(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let rel = Path::new(name);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "outside the state directory",
        ));
    }
    Ok(dir.join(rel))
}

impl Files for StoreFiles {
    fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.path(name)?)
    }

    fn remove(&self, name: &str) -> std::io::Result<()> {
        match std::fs::remove_file(self.path(name)?) {
            Err(x) if x.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}

impl Store {
    pub fn files(&self) -> StoreFiles {
        StoreFiles(self.dir.clone())
    }
}
