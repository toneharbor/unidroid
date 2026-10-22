use super::*;

pub(super) const MODES: [(bool, bool); 3] = [(true, false), (false, false), (false, true)];
pub(super) const EXECUTED: [(u64, u64); 7] =
    [(0, 0), (1, 1), (3, 2), (0, 0), (1, 1), (3, 2), (6, 3)];

pub(super) struct Vm {
    pub emu: Emulator,
    pub events: Events,
    pub entry: u64,
    pub dispatch: u64,
    pub program: u64,
    pub table: u64,
    pub reg: &'static str,
}
impl Vm {
    pub fn new(arm64: bool, thumb: bool) -> Result<Self> {
        // vpc = arg0; counter = 2; dispatch: load opcode; jump through arg1's table.
        // Opcodes 0/1 advance vpc by 1/2. Opcode 2 decrements the counter and
        // branches by its signed byte operand, or advances 3. Opcode 3 returns 42.
        let words: Vec<u32> = if arm64 {
            vec![
                0xaa0003f4, 0xd2800042, 0x39400283, 0xf8637823, 0xd61f0060, 0x91000694, 0x17fffffc,
                0x91000a94, 0x17fffffa, 0xf1000442, 0x54000080, 0x39800683, 0x8b030294, 0x17fffff5,
                0x91000e94, 0x17fffff3, 0xd2800540, 0xd65f03c0,
            ]
        } else if thumb {
            vec![
                0x22024604, 0xf8517823, 0x47183023, 0xe7f93401, 0xe7f73402, 0xd0033a01, 0x3001f994,
                0xe7f1441c, 0xe7ef3403, 0x4770202a,
            ]
        } else {
            vec![
                0xe1a04000, 0xe3a02002, 0xe5d43000, 0xe7913103, 0xe12fff13, 0xe2844001, 0xeafffffa,
                0xe2844002, 0xeafffff8, 0xe2522001, 0x0a000002, 0xe1d430d1, 0xe0844003, 0xeafffff3,
                0xe2844003, 0xeafffff1, 0xe3a0002a, 0xe12fff1e,
            ]
        };
        let (mut emu, events) = emulator(arm64, &[])?;
        let code = emu.modules()[0].base + 0x1000;
        emu.call_function(emu.resolve("mprotect")?, &[code, 4096, 7])?;
        emu.write_bytes(
            code,
            &words
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
        let program = emu.malloc(16)?;
        emu.write_bytes(program, &[0, 1, 0xa5, 2, 0xfd, 0, 3])?;
        let table = emu.malloc(32)?;
        let width = if arm64 { 8 } else { 4 };
        let offsets = if thumb {
            [12, 16, 20, 36]
        } else {
            [20, 28, 36, 64]
        };
        for (i, offset) in offsets.into_iter().enumerate() {
            let handler = (code + offset) | u64::from(thumb);
            emu.write_bytes(table + (i * width) as u64, &handler.to_le_bytes()[..width])?;
        }
        events.borrow_mut().clear();
        Ok(Self {
            emu,
            events,
            entry: code | u64::from(thumb),
            dispatch: code + if thumb { 4 } else { 8 },
            program,
            table,
            reg: if arm64 { "x20" } else { "r4" },
        })
    }

    pub fn tracepoint(&mut self, log: bool) -> Result<()> {
        self.emu.add_tracepoint(
            self.dispatch,
            &Tracepoint {
                values: vec![self.reg.into(), format!("[{}]:1", self.reg)],
                log,
            },
        )
    }

    pub fn run(&mut self, handler: &mut dyn JniHandler) -> Result<u64> {
        self.emu
            .call_function_with(handler, self.entry, &[self.program, self.table])
    }

    pub fn counts(&self, times: u64) -> Vec<(u64, u64)> {
        vec![
            (self.program, 2 * times),
            (self.program + 1, 2 * times),
            (self.program + 3, 2 * times),
            (self.program + 6, times),
        ]
    }
}

#[test]
fn bytecode_trace_and_coverage_follow_variable_width_instructions_and_loops() -> Result<()> {
    for (arm64, thumb) in MODES {
        let mut vm = Vm::new(arm64, thumb)?;
        // Warm the translation cache before adding the tracepoint.
        assert_eq!(vm.run(&mut ())?, 42);
        vm.tracepoint(true)?;
        assert_eq!(
            vm.run(&mut Handler(|_: &mut Emulator, _: &Stop| panic!(
                "trace stopped"
            )))?,
            42
        );
        let captured = vm.events.borrow();
        assert_eq!(captured.len(), EXECUTED.len());
        for ((event, _), (offset, opcode)) in captured.iter().zip(EXECUTED) {
            assert_eq!(event["event"], "tracepoint");
            assert_eq!(event["at"], vm.emu.symbolize(vm.dispatch));
            assert_eq!(event["address"], format!("0x{:x}", vm.dispatch));
            assert_eq!(
                event["values"],
                json!([
                    [vm.reg, format!("0x{:x}", vm.program + offset)],
                    [format!("[{}]:1", vm.reg), format!("0x{opcode:x}")]
                ])
            );
        }
        drop(captured);
        assert_eq!(vm.emu.tracepoint_hits(vm.dispatch)?, vm.counts(1));
        vm.tracepoint(false)?;
        assert!(vm.emu.tracepoint_hits(vm.dispatch)?.is_empty());
        vm.events.borrow_mut().clear();
        assert_eq!(vm.run(&mut ())?, 42);
        assert!(vm.events.borrow().is_empty());
        assert_eq!(vm.emu.tracepoint_hits(vm.dispatch)?, vm.counts(1));
        vm.emu.reset_tracepoint_hits();
        assert!(vm.emu.tracepoint_hits(vm.dispatch)?.is_empty());
    }
    Ok(())
}

#[test]
fn continue_at_dispatch_steps_exactly_one_bytecode_without_duplicate_trace_hits() -> Result<()> {
    for (arm64, thumb) in MODES {
        for breakpoint_first in [false, true] {
            let mut vm = Vm::new(arm64, thumb)?;
            if breakpoint_first {
                vm.emu.add_breakpoint(vm.dispatch)?;
            }
            vm.tracepoint(true)?;
            if !breakpoint_first {
                vm.emu.add_breakpoint(vm.dispatch)?;
            }
            let mut stops = Vec::new();
            let reg = vm.reg;
            let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.reason, StopReason::Breakpoint);
                stops.push(emu.register(reg).unwrap());
                assert!(stops.len() <= 7);
                Resume::Continue
            });
            assert_eq!(vm.run(&mut handler)?, 42);
            assert_eq!(stops, EXECUTED.map(|(offset, _)| vm.program + offset));
            assert_eq!(vm.events.borrow().len(), 7);
            assert_eq!(vm.emu.tracepoint_hits(vm.dispatch)?, vm.counts(1));
        }
    }
    Ok(())
}
