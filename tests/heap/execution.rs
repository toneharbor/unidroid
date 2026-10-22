use super::*;
use unidroid::{JniHandler, Resume, Stop};

struct Vm(u64);
impl JniHandler for Vm {
    fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
        self.0 = emu.registers().unwrap()[0].1;
        Resume::Return(0x10006)
    }
}

fn jni(emu: &mut Emulator, env: u64, arm64: bool, index: u64, args: [u64; 3]) -> Result<u64> {
    let width = if arm64 { 8 } else { 4 };
    let table = pointer(emu, env, arm64)?;
    let function = pointer(emu, table + index * width, arm64)?;
    let at = code(
        emu,
        0x20200,
        if arm64 {
            &[0xa9bf7bfd, 0xd63f0080, 0xa8c17bfd, 0xd65f03c0]
        } else {
            &[0xe59dc000, 0xe92d4010, 0xe12fff3c, 0xe8bd8010]
        },
    )?;
    emu.call_function(at, &[env, args[0], args[1], args[2], function])
}

#[test]
fn native_jni_string_and_array_buffers_have_allocation_and_free_backtraces() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let abi = if arm64 { "arm64-v8a" } else { "armeabi-v7a" };
        let module = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let onload = emu.find_symbol(module, "JNI_OnLoad")?;
        emu.add_breakpoint(onload)?;
        let mut vm = Vm(0);
        emu.call_jni_onload(module, &mut vm)?;
        emu.remove_breakpoint(onload)?;
        let width = if arm64 { 8 } else { 4 };
        let table = pointer(&emu, vm.0, arm64)?;
        let get_env = pointer(&emu, table + 6 * width, arm64)?;
        let out = emu.malloc(8)?;
        assert_eq!(emu.call_function(get_env, &[vm.0, out, 0x10006])?, 0);
        let env = pointer(&emu, out, arm64)?;
        let string = emu.new_string("hello");
        let array = emu.new_byte_array(&[1, 2, 3]);
        emu.reset_heap();
        for (get, release, handle, bytes) in [
            (169, 170, string, b"hello\0".to_vec()),
            (165, 166, string, b"h\0e\0l\0l\0o\0".to_vec()),
            (224, 225, string, b"h\0e\0l\0l\0o\0".to_vec()),
            (184, 192, array, vec![1, 2, 3]),
            (222, 223, array, vec![1, 2, 3]),
        ] {
            let address = jni(&mut emu, env, arm64, get, [handle.0, 0, 0])?;
            assert_eq!(emu.read_bytes(address, bytes.len())?, bytes);
            let allocation = emu.allocation(address).unwrap();
            assert_eq!(allocation.size, bytes.len() as u64);
            assert_eq!(allocation.function, "malloc");
            assert_eq!(allocation.backtrace[0].address, 0x20000);
            assert_eq!(
                allocation.backtrace[0].symbol.as_deref(),
                Some("host return")
            );
            jni(&mut emu, env, arm64, release, [handle.0, address, 2])?;
            let released = emu.allocation(address).unwrap();
            assert!(!released.live);
            assert_eq!(released.freed.unwrap()[0].address, 0x20000);
        }
        assert_eq!(emu.heap().len(), 5);
    }
    Ok(())
}

#[test]
fn guest_threads_pair_their_own_malloc_and_free_calls() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let worker = code(
            &mut emu,
            0x20200,
            if arm64 {
                &[
                    0xa9be7bfd, 0xf9000bf3, 0x910003fd, 0xaa0003f3, 0xf9400263, 0xd2800920,
                    0xd63f0060, 0xf9000a60, 0xf9400663, 0xd63f0060, 0xd2800000, 0xf9400bf3,
                    0xa8c27bfd, 0xd65f03c0,
                ]
            } else {
                &[
                    0xe92d4830, 0xe28db008, 0xe1a04000, 0xe5943000, 0xe3a00049, 0xe12fff33,
                    0xe5840008, 0xe5943004, 0xe12fff33, 0xe3a00000, 0xe8bd8830,
                ]
            },
        )?;
        let width = if arm64 { 8 } else { 4 };
        let malloc = emu.resolve("malloc")?;
        let free = emu.resolve("free")?;
        let create = emu.resolve("pthread_create")?;
        let join = emu.resolve("pthread_join")?;
        let out = emu.malloc(8)?;
        let mut threads = Vec::new();
        for _ in 0..2 {
            let args = emu.malloc(24)?;
            write_pointer(&mut emu, args, malloc, arm64)?;
            write_pointer(&mut emu, args + width, free, arm64)?;
            assert_eq!(emu.call_function(create, &[out, 0, worker, args])?, 0);
            threads.push(pointer(&emu, out, arm64)?);
        }
        let parent = emu.call_function(malloc, &[73])?;
        for thread in threads {
            assert_eq!(emu.call_function(join, &[thread, 0])?, 0);
        }
        let records: Vec<_> = emu.heap().into_iter().filter(|a| a.size == 73).collect();
        assert_eq!(records.len(), 3);
        let children: Vec<_> = records.iter().filter(|a| a.thread != 1234).collect();
        assert_eq!(children.len(), 2);
        assert_ne!(children[0].thread, children[1].thread);
        for child in children {
            assert!(!child.live);
            assert_eq!(
                child.backtrace[0].address,
                if arm64 { 0x2021c } else { 0x20218 }
            );
            assert_eq!(
                child.freed.as_ref().unwrap()[0].address,
                if arm64 { 0x20228 } else { 0x20224 }
            );
        }
        assert!(emu.allocation(parent).unwrap().live);
        assert_eq!(emu.allocation(parent).unwrap().thread, 1234);
    }
    Ok(())
}

#[test]
fn a_debugger_nested_allocation_does_not_complete_or_duplicate_its_caller() -> Result<()> {
    struct Nested {
        malloc: u64,
        inner: u64,
    }
    impl JniHandler for Nested {
        fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
            self.inner = emu.call_function(self.malloc, &[48]).unwrap();
            Resume::Continue
        }
    }
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let malloc = emu.resolve("malloc")?;
        emu.add_breakpoint(malloc)?;
        let mut handler = Nested { malloc, inner: 0 };
        let outer = emu.call_function_with(&mut handler, malloc, &[32])?;
        emu.remove_breakpoint(malloc)?;
        let records = emu.heap();
        assert_eq!(records.len(), 2);
        assert_eq!((records[0].address, records[0].size), (outer, 32));
        assert_eq!((records[1].address, records[1].size), (handler.inner, 48));
    }
    Ok(())
}

#[test]
fn aborted_allocator_calls_do_not_leave_pending_allocations_for_the_next_call() -> Result<()> {
    struct Abort;
    impl JniHandler for Abort {
        fn stopped(&mut self, _: &mut Emulator, _: &Stop) -> Resume {
            Resume::Abort
        }
    }
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let malloc = emu.resolve("malloc")?;
        emu.add_breakpoint(malloc)?;
        assert!(emu.call_function_with(&mut Abort, malloc, &[32]).is_err());
        emu.remove_breakpoint(malloc)?;
        let address = emu.call_function(malloc, &[64])?;
        let records = emu.heap();
        assert_eq!(records.len(), 1);
        assert_eq!((records[0].address, records[0].size), (address, 64));
    }
    Ok(())
}

#[test]
fn trace_enabled_before_cpu_creation_and_repeated_loads_install_each_export_once() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = Emulator::builder().build()?;
        emu.set_trace(Trace {
            heap: true,
            ..Trace::default()
        })?;
        let lib = if arm64 {
            "rootfs/lib64/libc.so"
        } else {
            "rootfs/lib/libc.so"
        };
        emu.load_library(if arm64 {
            "rootfs/lib64/libdl.so"
        } else {
            "rootfs/lib/libdl.so"
        })?;
        assert!(!emu.modules().iter().any(|module| module.name == "libc.so"));
        emu.load_library(lib)?;
        emu.load_library(lib)?;
        emu.reset_heap();
        let malloc = emu.resolve("malloc")?;
        emu.call_function(malloc, &[32])?;
        assert_eq!(emu.heap().len(), 1);
    }
    Ok(())
}
