use super::*;
use crate::Architecture;

#[test]
fn updating_one_tracepoint_keeps_other_hooks_installed() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let point = Tracepoint {
            values: vec!["0x1".into()],
            log: false,
        };
        emu.add_tracepoint(0x20100, &point)?;
        let first = emu.tracepoints.points[&0x20100].1;
        for address in [0x20104, 0x20108, 0x20104] {
            emu.add_tracepoint(address, &point)?;
            assert_eq!(emu.tracepoints.points[&0x20100].1, first);
        }
        emu.remove_tracepoint(0x20104)?;
        assert_eq!(emu.tracepoints.points[&0x20100].1, first);
        emu.remove_tracepoint(0x20108)?;
        assert_eq!(emu.tracepoints.points.len(), 1);
        assert_eq!(emu.tracepoints.points[&0x20100].1, first);
    }
    Ok(())
}

#[test]
fn tracepoints_refresh_after_added_code_hooks_only() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let events = std::rc::Rc::new(std::cell::RefCell::new(0));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                if matches!(event, Event::Tracepoint { .. }) {
                    *sink.borrow_mut() += 1;
                }
            })
            .build()?;
        emu.setup_cpu(arch)?;
        let code = crate::RETURN + 0x100;
        let words = match arch {
            Architecture::Arm32 => [0xe3a0002au32, 0xe12fff1e],
            Architecture::Arm64 => [0xd2800540, 0xd65f03c0],
        };
        emu.write_bytes(
            code,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        emu.add_tracepoint(
            code,
            &Tracepoint {
                values: vec!["0x1".into()],
                log: true,
            },
        )?;
        let first = emu.tracepoints.points[&code].1;
        assert_eq!(emu.call_function(code, &[])?, 42);
        assert_eq!(emu.tracepoints.points[&code].1, first);
        assert_eq!(*events.borrow(), 1);

        emu.reset_tracepoint_hits();
        *events.borrow_mut() = 0;
        emu.add_breakpoint(code)?;
        let mut stops = Vec::new();
        let mut handler = TestHandler(&mut stops);
        assert_eq!(emu.call_function_with(&mut handler, code, &[])?, 42);
        let second = emu.tracepoints.points[&code].1;
        assert_ne!(second, first);
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].address, code);
        assert_eq!(emu.tracepoint_hits(code)?, [(1, 1)]);
        assert_eq!(*events.borrow(), 1);
    }
    Ok(())
}

struct TestHandler<'a>(&'a mut Vec<crate::Stop>);
impl crate::JniHandler for TestHandler<'_> {
    fn stopped(&mut self, _: &mut Emulator, stop: &crate::Stop) -> crate::Resume {
        self.0.push(*stop);
        crate::Resume::Continue
    }
}

#[test]
fn tracepoint_symbolization_uses_modules_at_hit_time() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                if let Event::Tracepoint { at, .. } = event {
                    sink.borrow_mut().push(at.clone());
                }
            })
            .build()?;
        let library_a = crate::test_elf::TestLibrary::new(arch == Architecture::Arm64, false);
        let library_b = crate::test_elf::TestLibrary::new(arch == Architecture::Arm64, false);
        emu.load_library(&library_a.0)?;
        emu.add_tracepoint(
            0x1001_0200,
            &Tracepoint {
                values: vec!["0x1".into()],
                log: true,
            },
        )?;
        emu.load_library(&library_b.0)?;
        assert_eq!(emu.call_function(0x1001_0200, &[])?, 42);
        let name = library_b.0.file_name().unwrap().to_string_lossy();
        assert_eq!(
            events.borrow().as_slice(),
            [format!("{name}+0x200 (answer)")]
        );
    }
    Ok(())
}
