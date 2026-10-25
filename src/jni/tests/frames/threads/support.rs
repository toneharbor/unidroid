use super::*;

pub(super) struct AtStop<F>(pub F);
impl<F: FnMut(&mut Emulator, &Stop) -> Result<Resume>> JniHandler for AtStop<F> {
    fn stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume {
        (self.0)(emu, stop).unwrap_or_else(|error| panic!("{error}"))
    }
}

pub(super) const SPIN: u64 = CODE + 0x100;

pub(super) struct Worker {
    pub thread: u64,
    pub flags: u64,
    pub finish: u64,
}
impl Worker {
    pub fn spawn(emu: &mut Emulator, libc: Module) -> Result<Self> {
        let arm32 = emu.architecture() == Architecture::Arm32;
        // Stop to create locals, signal ready, then wait for the host's flag.
        // Both spins are bounded at 50,000 tries; no wall-clock timing is used.
        let worker: &[u32] = if arm32 {
            &[
                0xe320f000, // nop (first stop)
                0xe3a01042, // mov r1,#0x42
                0xe5801000, // str r1,[r0]
                0xe30c2350, // movw r2,#50000
                0xe5901004, // wait: ldr r1,[r0,#4]
                0xe3510000, // cmp r1,#0
                0x1a000001, // bne finish
                0xe2522001, // subs r2,r2,#1
                0x1afffffa, // bne wait
                0xe320f000, // finish: nop (second stop)
                0xe3a00000, // mov r0,#0
                0xe12fff1e, // bx lr
            ]
        } else {
            &[
                0xd503201f, // nop (first stop)
                0x52800841, // mov w1,#0x42
                0xb9000001, // str w1,[x0]
                0x52986a02, // mov w2,#50000
                0xb9400401, // wait: ldr w1,[x0,#4]
                0x35000061, // cbnz w1,finish
                0x71000442, // subs w2,w2,#1
                0x54ffffa1, // b.ne wait
                0xd503201f, // finish: nop (second stop)
                0xd2800000, // mov x0,#0
                0xd65f03c0, // ret
            ]
        };
        write_code(emu, worker)?;
        let spin: &[u32] = if arm32 {
            &[
                0xe30c2350, 0xe5901000, 0xe3510000, 0x1a000001, 0xe2522001, 0x1afffffa, 0xe1a00001,
                0xe12fff1e,
            ]
        } else {
            &[
                0x52986a02, 0xb9400001, 0x35000061, 0x71000442, 0x54ffffa1, 0x2a0103e0, 0xd65f03c0,
            ]
        };
        emu.write_bytes(
            SPIN,
            &spin
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
        let finish = CODE + if arm32 { 36 } else { 32 };
        emu.add_breakpoint(CODE)?;
        emu.add_breakpoint(finish)?;
        let flags = emu.malloc(8)?;
        let out = emu.malloc(8)?;
        let create = emu.find_symbol(libc, "pthread_create")?;
        assert_eq!(emu.call_function(create, &[out, 0, CODE, flags])?, 0);
        Ok(Self {
            thread: emu.read_pointer(out)?,
            flags,
            finish,
        })
    }

    pub fn wait(&self, emu: &mut Emulator, handler: &mut dyn JniHandler) -> Result<()> {
        assert_eq!(emu.call_function_with(handler, SPIN, &[self.flags])?, 0x42);
        Ok(())
    }

    pub fn join(
        &self,
        emu: &mut Emulator,
        libc: Module,
        handler: &mut dyn JniHandler,
    ) -> Result<()> {
        emu.write_bytes(self.flags + 4, &1u32.to_le_bytes())?;
        let join = emu.find_symbol(libc, "pthread_join")?;
        assert_eq!(emu.call_function_with(handler, join, &[self.thread, 0])?, 0);
        Ok(())
    }

    pub fn released(&self, emu: &Emulator) -> Result<()> {
        assert_eq!(emu.read_bytes(self.flags + 4, 4)?, 1u32.to_le_bytes());
        Ok(())
    }
}
