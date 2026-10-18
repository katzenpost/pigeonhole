//! Members, each a client engine, over the in-memory Pigeonhole, on a logical clock that moves
//! only when told to: as group_chat_client's client scenarios and core tests drive them.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pigeonhole_katzenqt1_client::{Executor, Files, Outcome};
use pigeonhole_katzenqt1_engine::{Action, Chat, Engine, Input, Line, State, Status, plain};
use pigeonhole_katzenqt1_member::Key;
use pigeonhole_katzenqt1_wire as wire;
use pigeonhole_transport::sim::World as Sim;

#[derive(Clone, Default)]
pub struct MemFiles(pub Arc<Mutex<HashMap<String, Vec<u8>>>>);

impl Files for MemFiles {
    fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        self.0
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, name.to_string()))
    }

    fn remove(&self, name: &str) -> std::io::Result<()> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

pub struct Member {
    pub name: String,
    pub eng: Engine,
    pub files: MemFiles,
}

pub enum Later {
    Act(Action),
    In(Input),
}

pub struct World {
    pub sim: Sim,
    pub members: Vec<Member>,
    pub queue: Vec<(usize, Action)>,
    pub later: Vec<(u64, usize, Later)>,
    pub clock: u64,
    pub held: HashSet<usize>,
    pub epoch: u64,
    rt: tokio::runtime::Runtime,
}

impl Default for World {
    fn default() -> Self {
        World {
            sim: Sim::default(),
            members: Vec::new(),
            queue: Vec::new(),
            later: Vec::new(),
            clock: 0,
            held: HashSet::new(),
            epoch: 1,
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
        }
    }
}

impl World {
    pub fn who(&mut self, name: &str) -> usize {
        if let Some(i) = self.members.iter().position(|m| m.name == name) {
            return i;
        }
        self.members.push(Member {
            name: name.into(),
            eng: Engine::new(
                State {
                    me: name.into(),
                    chats: Vec::new(),
                },
                1530,
            ),
            files: MemFiles::default(),
        });
        self.members.len() - 1
    }

    pub fn feed(&mut self, i: usize, input: Input) {
        let res = self.members[i].eng.step(0, input);
        let mut files = self.members[i].files.0.lock().unwrap();
        for f in res.files {
            files.insert(f.file, f.bytes);
        }
        drop(files);
        self.queue.extend(res.actions.into_iter().map(|a| (i, a)));
    }

    pub fn act(&mut self) -> bool {
        let Some(k) = self.queue.iter().position(|q| !self.held.contains(&q.0)) else {
            return false;
        };
        let (who, a) = self.queue.remove(k);
        let x = Executor {
            backend: self.sim.clone(),
            files: self.members[who].files.clone(),
        };
        let o = self.rt.block_on(x.perform(a, |_| async {}));
        match o {
            Outcome::Retry(a, ms) => self.later.push((self.clock + ms, who, Later::Act(a))),
            Outcome::Later(input, ms) => self.later.push((self.clock + ms, who, Later::In(input))),
            Outcome::Done(input) => self.feed(who, input),
            Outcome::Nothing => {}
        }
        true
    }

    /// Let the network run until `stop` on the logical clock.
    pub fn run_to(&mut self, stop: u64) {
        loop {
            if self.act() {
                continue;
            }
            let due = self
                .later
                .iter()
                .filter(|l| l.0 <= stop && !self.held.contains(&l.1))
                .map(|l| l.0)
                .min();
            let Some(due) = due else {
                self.clock = self.clock.max(stop);
                return;
            };
            let (now, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.later)
                .into_iter()
                .partition(|l| l.0 <= due && !self.held.contains(&l.1));
            self.later = rest;
            self.clock = self.clock.max(due);
            for (_, who, l) in now {
                match l {
                    Later::Act(a) => self.queue.push((who, a)),
                    Later::In(input) => self.feed(who, input),
                }
            }
        }
    }

    /// Run one action, or move the clock on to the next thing due; false when there is nothing
    /// left at all.
    pub fn tick(&mut self) -> bool {
        if self.act() {
            return true;
        }
        let Some(due) = self
            .later
            .iter()
            .filter(|l| !self.held.contains(&l.1))
            .map(|l| l.0)
            .min()
        else {
            return false;
        };
        self.run_to(due);
        true
    }

    /// Run until `done` holds or the clock passes `limit` milliseconds from now.
    pub fn run_until(&mut self, done: impl Fn(&World) -> bool, limit: u64) -> bool {
        let start = self.clock;
        while !done(self) {
            if self.clock > start + limit || !self.tick() {
                return done(self);
            }
        }
        true
    }

    pub fn chat(&self, i: usize, group: &str) -> Option<&Chat> {
        self.members[i].eng.state.chat(group)
    }

    /// The member whose key `k` is: the truth, for printing.
    pub fn name_of(&self, k: &Key) -> String {
        for m in &self.members {
            for c in &m.eng.state.chats {
                if c.group.as_ref().is_some_and(|g| &g.me() == k) {
                    return m.name.clone();
                }
            }
        }
        "?".into()
    }

    pub fn code(&self, i: usize, group: &str) -> String {
        self.chat(i, group)
            .and_then(|c| c.joining.as_ref())
            .map_or(String::new(), |j| j.code.clone())
    }

    pub fn says(&self, i: usize, chat: &str, text: &str) -> bool {
        self.chat(i, chat)
            .is_some_and(|c| c.log.iter().any(|l| l.notice.is_none() && l.text == text))
    }

    pub fn noticed(&self, i: usize, chat: &str, text: &str) -> bool {
        self.chat(i, chat).is_some_and(|c| {
            c.log.iter().any(|l| {
                l.notice
                    .as_ref()
                    .is_some_and(|n| plain(n).starts_with(text))
            })
        })
    }

    pub fn joined(&self, i: usize, chat: &str) -> bool {
        self.chat(i, chat).is_some_and(|c| c.group.is_some())
    }

    pub fn idle(&self) -> bool {
        self.members
            .iter()
            .all(|m| m.eng.state.chats.iter().all(|c| c.jobs.is_empty()))
    }

    fn first_of(c: &Chat) -> u64 {
        wire::counter(&c.write_cap[c.write_cap.len() - wire::INDEX_SIZE..])
    }

    fn mark(c: &Chat, l: &Line) -> &'static str {
        match l.status {
            Status::Pending => " …",
            Status::Sent if c.acknowledged(l) => " ✓✓",
            Status::Sent => " ✓",
        }
    }

    pub fn show(&self, i: usize, group: &str) -> Vec<String> {
        let Some(c) = self.chat(i, group) else {
            return vec!["no such group".into()];
        };
        c.log
            .iter()
            .map(|l| match &l.notice {
                Some(n) => format!("— {}", plain(n)),
                None => {
                    let mine = c.group.as_ref().is_some_and(|g| g.me() == l.sender);
                    format!(
                        "{}{}: {}",
                        self.name_of(&l.sender),
                        if mine { Self::mark(c, l) } else { "" },
                        l.text.replace('\n', " | ")
                    )
                }
            })
            .collect()
    }

    pub fn members_of(&self, i: usize, group: &str) -> Vec<String> {
        let Some(c) = self.chat(i, group) else {
            return vec!["no such group".into()];
        };
        let Some(g) = &c.group else {
            return vec!["not joined".into()];
        };
        let mut names: Vec<String> = g.readers.keys().map(|k| self.name_of(k)).collect();
        names.sort();
        let s = names.join(" ").trim().to_string();
        vec![if s.is_empty() { "nobody".into() } else { s }]
    }

    pub fn acked(&self, i: usize, group: &str) -> Vec<String> {
        let Some(c) = self.chat(i, group) else {
            return vec!["no such group".into()];
        };
        let Some(g) = &c.group else {
            return vec!["not joined".into()];
        };
        let first = Self::first_of(c);
        let mut acks: Vec<String> = g
            .ledger
            .acked_by
            .iter()
            .map(|(k, n)| format!("{}:{}", self.name_of(k), n - first))
            .collect();
        acks.sort();
        vec![if acks.is_empty() {
            "none".into()
        } else {
            acks.join(" ")
        }]
    }

    pub fn state_of(&self, i: usize, group: &str) -> Vec<String> {
        let Some(c) = self.chat(i, group) else {
            return vec!["no such group".into()];
        };
        let mut parts = vec![if c.group.is_some() {
            "joined".to_string()
        } else {
            "not joined".to_string()
        }];
        if let Some(j) = &c.joining {
            parts.push(if j.code.is_empty() {
                "minting".into()
            } else {
                "awaiting a reply".into()
            });
        }
        if c.inducting.is_some() {
            parts.push("adding a member".into());
        }
        if !c.jobs.is_empty() {
            parts.push(format!("{} to write", c.jobs.len()));
        }
        vec![parts.join(", ")]
    }

    pub fn jobs(&self, i: usize, group: &str) -> Vec<String> {
        let Some(c) = self.chat(i, group) else {
            return vec!["no such group".into()];
        };
        if c.jobs.is_empty() {
            return vec!["none".into()];
        }
        let first = Self::first_of(c);
        c.jobs
            .iter()
            .map(|j| {
                let at = j.index.as_ref().map_or("next box".into(), |x| {
                    format!("box {}", wire::counter(x) - first)
                });
                let what = if j.token.is_some() {
                    "message"
                } else if j.box_.is_empty() {
                    "tombstone"
                } else {
                    "rewrite"
                };
                format!("{what} to {at}, {} bytes", j.box_.len())
            })
            .collect()
    }

    pub fn refused(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for r in &self.sim.state().refused {
            let (mut name, mut n) = ("?".to_string(), r.idx64);
            for m in &self.members {
                for c in &m.eng.state.chats {
                    if c.write_cap.len() > 64 && c.write_cap[32..64] == r.stream[..] {
                        name = m.name.clone();
                        n = r.idx64 - Self::first_of(c);
                    }
                }
            }
            let s = format!("{name}@{n}");
            if !names.contains(&s) {
                names.push(s);
            }
        }
        vec![if names.is_empty() {
            "none".into()
        } else {
            names.join(" ")
        }]
    }

    pub fn command(&mut self, words: &[&str]) -> Result<Vec<String>, String> {
        let id = |g: &str| g.to_string();
        match words {
            ["create", w, g] => {
                let i = self.who(w);
                self.feed(
                    i,
                    Input::Create {
                        chat: id(g),
                        name: id(g),
                    },
                );
            }
            ["join", w, g] => {
                let i = self.who(w);
                self.feed(
                    i,
                    Input::Join {
                        chat: id(g),
                        name: id(g),
                    },
                );
            }
            ["invite", w, g, new, rest @ ..] => {
                let theirs = rest.first().copied().unwrap_or(g);
                let n = self.who(new);
                let code = self.code(n, theirs);
                if code.is_empty() {
                    return Ok(vec![format!("{new} has no voucher code")]);
                }
                let i = self.who(w);
                self.feed(i, Input::Induct { chat: id(g), code });
            }
            ["send", w, g, text @ ..] => {
                let i = self.who(w);
                self.feed(
                    i,
                    Input::Send {
                        chat: id(g),
                        text: text.join(" "),
                    },
                );
            }
            ["remove", w, g, gone] => {
                let i = self.who(w);
                let n = self.who(gone);
                let Some(k) = self
                    .chat(n, g)
                    .and_then(|c| c.group.as_ref())
                    .map(|g| g.me())
                else {
                    return Ok(vec![format!("{gone} is not in {g}")]);
                };
                self.feed(
                    i,
                    Input::RemoveMember {
                        chat: id(g),
                        who: k,
                    },
                );
            }
            ["scan", w, g] => {
                let i = self.who(w);
                self.feed(i, Input::Scan { chat: id(g) });
            }
            ["leave", w, g] => {
                let i = self.who(w);
                self.feed(i, Input::Leave { chat: id(g) });
            }
            ["run"] => self.run_to(self.clock + 600000),
            ["run", s] => self.run_to(self.clock + s.parse::<u64>().unwrap() * 1000),
            ["hold", w] => {
                let i = self.who(w);
                self.held.insert(i);
            }
            ["release", w] => {
                let i = self.who(w);
                self.held.remove(&i);
            }
            ["epoch"] => {
                self.epoch += 1;
                self.sim.set_epoch(self.epoch);
                for i in 0..self.members.len() {
                    self.feed(
                        i,
                        Input::Epoch {
                            n: self.epoch as usize,
                        },
                    );
                }
            }
            ["fail-writes", n] => self.sim.state().fail_writes = n.parse().unwrap(),
            ["show", w, g] => {
                let i = self.who(w);
                return Ok(self.show(i, g));
            }
            ["members", w, g] => {
                let i = self.who(w);
                return Ok(self.members_of(i, g));
            }
            ["acked", w, g] => {
                let i = self.who(w);
                return Ok(self.acked(i, g));
            }
            ["state", w, g] => {
                let i = self.who(w);
                return Ok(self.state_of(i, g));
            }
            ["jobs", w, g] => {
                let i = self.who(w);
                return Ok(self.jobs(i, g));
            }
            ["refused"] => return Ok(self.refused()),
            _ => return Err(format!("not a command: {}", words.join(" "))),
        }
        Ok(Vec::new())
    }

    /// Run a scenario: the script with every checked command's output replaced by what it printed
    /// now.
    pub fn run_script(&mut self, script: &str) -> Result<String, String> {
        let mut lines: Vec<&str> = script.split('\n').collect();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        let mut out: Vec<String> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            out.push(line.into());
            i += 1;
            let cmd = line.split('#').next().unwrap_or("").trim();
            if cmd.is_empty() {
                continue;
            }
            let words: Vec<&str> = cmd.split_whitespace().collect();
            let printed = self.command(&words)?;
            if i < lines.len() && lines[i] == "----" {
                out.push("----".into());
                i += 1;
                while i < lines.len() && !lines[i].trim().is_empty() {
                    i += 1;
                }
                out.extend(printed);
            }
        }
        Ok(out.join("\n") + "\n")
    }
}
