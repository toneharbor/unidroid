use super::*;

use crate::test_apk as apk_support;
use apk_support::{TestApk, manifest};

mod assets;
mod dispatch;
mod exceptions;
mod lifecycle;
mod packages;
mod signatures;

#[test]
fn android_objects_have_the_same_text_directly_and_inside_lists() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        let bytes = emu.new_byte_array(b"ab");
        let signature = construct(
            &mut emu,
            "android/content/pm/Signature",
            "([B)V",
            &[bytes.into()],
        )?;
        assert_eq!(
            text(&mut emu, signature)?,
            "android.content.pm.Signature@fe2"
        );
        let list = emu.new_java_list("java/util/ArrayList", &[signature])?;
        let value = static_call(
            &mut emu,
            "java/lang/String",
            "valueOf(Ljava/lang/Object;)Ljava/lang/String;",
            &[list.into()],
        )?
        .object();
        assert_eq!(emu.string(value)?, "[android.content.pm.Signature@fe2]");
        let builder = construct(&mut emu, "java/lang/StringBuilder", "()V", &[])?;
        call(
            &mut emu,
            builder,
            "append(Ljava/lang/Object;)Ljava/lang/StringBuilder;",
            &[list.into()],
        )?;
        assert_eq!(
            text(&mut emu, builder)?,
            "[android.content.pm.Signature@fe2]"
        );
    }
    Ok(())
}

#[test]
fn android_superclasses_require_an_apk_unless_explicitly_declared() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        let class = emu.class("android/app/Application");
        let parent = Handle(jni_call(&mut emu, 10, &[class.0])?);
        assert_eq!(emu.class_name(parent)?, "android/content/ContextWrapper");
        let mut emu = emulator(arch)?;
        let class = emu.class("android/app/Application");
        assert!(
            jni_call(&mut emu, 10, &[class.0])
                .unwrap_err()
                .to_string()
                .contains("superclass not declared")
        );
        emu.superclass("android/app/Application", "app/Declared");
        let parent = Handle(jni_call(&mut emu, 10, &[class.0])?);
        assert_eq!(emu.class_name(parent)?, "app/Declared");
    }
    Ok(())
}

fn field(
    emu: &mut Emulator,
    receiver: Handle,
    class: &str,
    name: &str,
    signature: &str,
    is_static: bool,
) -> Result<JniValue> {
    field_with(emu, &mut (), receiver, class, name, signature, is_static)
}

fn field_with(
    emu: &mut Emulator,
    handler: &mut dyn JniHandler,
    receiver: Handle,
    class: &str,
    name: &str,
    signature: &str,
    is_static: bool,
) -> Result<JniValue> {
    let class = emu.class(class);
    let name = emu.new_cstring(name)?;
    let sig = emu.new_cstring(signature)?;
    let id = jni_call(emu, if is_static { 144 } else { 94 }, &[class.0, name, sig])?;
    emu.free(name);
    emu.free(sig);
    let kind = signature.as_bytes()[0];
    let slot = b"LZBCSIJFD"
        .iter()
        .position(|&b| b == if kind == b'[' { b'L' } else { kind })
        .unwrap();
    let index = (if is_static { 145 } else { 95 }) + slot as u64;
    let table = emu.read_pointer(emu.jni.env)?;
    let function = emu.read_pointer(table + index * emu.pointer_size())?;
    let bits = emu.call_function_with(
        handler,
        function,
        &[
            emu.jni.env,
            if is_static { class.0 } else { receiver.0 },
            id,
        ],
    )?;
    Ok(crate::jni::value::decode(kind, bits))
}

fn package_info(emu: &mut Emulator, flags: i32) -> Result<Handle> {
    let app = application(emu)?;
    let pm = call(
        emu,
        app,
        "getPackageManager()Landroid/content/pm/PackageManager;",
        &[],
    )?
    .object();
    let name = emu.new_string("com.example.apk");
    Ok(call(
        emu,
        pm,
        "getPackageInfo(Ljava/lang/String;I)Landroid/content/pm/PackageInfo;",
        &[name.into(), JniValue::Int(flags)],
    )?
    .object())
}

fn package_field(
    emu: &mut Emulator,
    receiver: Handle,
    name: &str,
    signature: &str,
) -> Result<JniValue> {
    field(
        emu,
        receiver,
        "android/content/pm/PackageInfo",
        name,
        signature,
        false,
    )
}

fn apk_emulator(arch: Architecture) -> Result<(TestApk, Emulator)> {
    let file = TestApk::manifest(&manifest::xml(true, &[manifest::package()]))?;
    let mut emu = Emulator::builder().apk(&file.path).build()?;
    emu.setup_cpu(arch)?;
    Ok((file, emu))
}

fn application(emu: &mut Emulator) -> Result<Handle> {
    Ok(static_call(
        emu,
        "android/app/ActivityThread",
        "currentApplication()Landroid/app/Application;",
        &[],
    )?
    .object())
}

#[test]
fn application_and_declared_contexts_answer_package_entry_points() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        let app = application(&mut emu)?;
        emu.superclass("app/Host", "android/app/Application");
        let host = emu.new_object("app/Host", 42);
        let direct = emu.new_object("android/app/Application", ());
        for receiver in [app, host, direct] {
            for (method, expected) in [
                ("getPackageName()Ljava/lang/String;", "com.example.apk"),
                (
                    "getPackageCodePath()Ljava/lang/String;",
                    "/data/app/com.example.apk-1/base.apk",
                ),
                (
                    "getPackageResourcePath()Ljava/lang/String;",
                    "/data/app/com.example.apk-1/base.apk",
                ),
            ] {
                let value = call(&mut emu, receiver, method, &[])?.object();
                assert_eq!(emu.string(value)?, expected);
            }
            for (method, class) in [
                (
                    "getPackageManager()Landroid/content/pm/PackageManager;",
                    "android.app.ApplicationPackageManager",
                ),
                (
                    "getApplicationInfo()Landroid/content/pm/ApplicationInfo;",
                    "android.content.pm.ApplicationInfo",
                ),
                (
                    "getAssets()Landroid/content/res/AssetManager;",
                    "android.content.res.AssetManager",
                ),
                (
                    "getApplicationContext()Landroid/content/Context;",
                    "android.app.Application",
                ),
            ] {
                let object = call(&mut emu, receiver, method, &[])?.object();
                let kind = call(&mut emu, object, "getClass()Ljava/lang/Class;", &[])?.object();
                let name = call(&mut emu, kind, "getName()Ljava/lang/String;", &[])?.object();
                assert_eq!(emu.string(name)?, class);
                if class == "android.app.Application" {
                    assert_ne!(object, app);
                    assert_eq!(jni_call(&mut emu, 24, &[object.0, app.0])?, 1);
                }
            }
        }
        assert_eq!(*emu.user_value::<i32>(host).unwrap(), 42);
        let opaque = emu.new_object("app/Unknown", ());
        let mut probe = crate::Probe::default();
        member(
            &mut emu,
            &mut probe,
            opaque,
            "app/Unknown",
            "getPackageName()Ljava/lang/String;",
            &[],
            0,
        )?;
        assert_eq!(probe.calls().len(), 1);
        let value = member(
            &mut emu,
            &mut (),
            opaque,
            "android/content/Context",
            "getPackageName()Ljava/lang/String;",
            &[],
            0,
        )?
        .object();
        assert_eq!(emu.string(value)?, "com.example.apk");
    }
    Ok(())
}
