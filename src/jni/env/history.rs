use crate::{Emulator, jni::names::jni_name};
use std::fmt::Write;

const RECENT_JNI: usize = 16;

#[derive(Clone, Copy, Default)]
struct Call {
    index: usize,
    lr: u64,
    thread: u32,
    member: Option<u64>,
    args: [u64; 3],
}

#[derive(Default)]
pub(crate) struct History {
    calls: [Call; RECENT_JNI],
    next: usize,
    len: usize,
}

impl History {
    pub(super) fn record(
        &mut self,
        index: usize,
        lr: u64,
        thread: u32,
        member: Option<u64>,
        args: [u64; 3],
    ) {
        self.calls[self.next] = Call {
            index,
            lr,
            thread,
            member,
            args,
        };
        self.next = (self.next + 1) % RECENT_JNI;
        self.len = (self.len + 1).min(RECENT_JNI);
    }
}

impl Emulator {
    pub(crate) fn recent_jni(&self) -> Vec<String> {
        let history = &self.jni.history;
        (0..history.len)
            .map(|i| {
                let call =
                    history.calls[(history.next + RECENT_JNI - history.len + i) % RECENT_JNI];
                let mut line = format!("JNIEnv->{}", jni_name(call.index));
                if let Some(id) = call.member
                    && let Ok(member) = self.require_member("", id)
                {
                    let _ = write!(
                        line,
                        " {}->{}{}",
                        member.class, member.name, member.signature
                    );
                } else if let Ok(Some(details)) = self.jni_details(call.index, call.args) {
                    let _ = write!(line, "({details})");
                }
                let _ = write!(line, " from {}", self.location(call.lr));
                if call.thread != self.threads.tid() {
                    let _ = write!(line, " in thread {}", call.thread);
                }
                line
            })
            .collect()
    }
}
