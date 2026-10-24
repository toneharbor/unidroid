use super::*;

#[test]
fn null_android_arguments_return_null_and_set_pending_null_pointer_exceptions() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        let app = application(&mut emu)?;
        let manager = call(
            &mut emu,
            app,
            "getPackageManager()Landroid/content/pm/PackageManager;",
            &[],
        )?
        .object();
        let assets = call(
            &mut emu,
            app,
            "getAssets()Landroid/content/res/AssetManager;",
            &[],
        )?
        .object();
        for (receiver, class, method, signature, constructor) in [
            (
                manager,
                "android/content/pm/PackageManager",
                "getPackageInfo",
                "(Ljava/lang/String;I)Landroid/content/pm/PackageInfo;",
                false,
            ),
            (
                manager,
                "android/content/pm/PackageManager",
                "getApplicationInfo",
                "(Ljava/lang/String;I)Landroid/content/pm/ApplicationInfo;",
                false,
            ),
            (
                assets,
                "android/content/res/AssetManager",
                "list",
                "(Ljava/lang/String;)[Ljava/lang/String;",
                false,
            ),
            (
                Handle(0),
                "android/content/pm/Signature",
                "<init>",
                "([B)V",
                true,
            ),
        ] {
            let class = emu.class(class);
            let name = emu.new_cstring(method)?;
            let sig = emu.new_cstring(signature)?;
            let id = jni_call(&mut emu, 33, &[class.0, name, sig])?;
            let slots = emu.malloc(16)?;
            emu.write_bytes(slots, &[0; 16])?;
            assert_eq!(
                jni_call(
                    &mut emu,
                    if constructor { 30 } else { 36 },
                    &[if constructor { class.0 } else { receiver.0 }, id, slots]
                )?,
                0
            );
            assert_eq!(jni_call(&mut emu, 228, &[])?, 1);
            let exception = jni_call(&mut emu, 15, &[])?;
            let kind = Handle(jni_call(&mut emu, 31, &[exception])?);
            jni_call(&mut emu, 17, &[])?;
            let value = call(&mut emu, kind, "getName()Ljava/lang/String;", &[])?.object();
            assert_eq!(emu.string(value)?, "java.lang.NullPointerException");
            for pointer in [name, sig, slots] {
                emu.free(pointer);
            }
        }
    }
    Ok(())
}

#[test]
fn uncovered_android_fields_and_signatures_reach_the_handler() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        let info = package_info(&mut emu, 0)?;
        let application_info = package_field(
            &mut emu,
            info,
            "applicationInfo",
            "Landroid/content/pm/ApplicationInfo;",
        )?
        .object();
        let mut probe = crate::Probe::default();
        for (receiver, class, name, signature, is_static) in [
            (
                info,
                "android/content/pm/PackageInfo",
                "uncovered",
                "I",
                false,
            ),
            (
                info,
                "android/content/pm/PackageInfo",
                "versionCode",
                "Ljava/lang/String;",
                false,
            ),
            (
                application_info,
                "android/content/pm/ApplicationInfo",
                "uid",
                "I",
                false,
            ),
            (
                Handle(0),
                "android/content/pm/PackageManager",
                "uncovered",
                "I",
                true,
            ),
            (
                Handle(0),
                "android/content/pm/PackageManager",
                "GET_SIGNATURES",
                "Ljava/lang/String;",
                true,
            ),
        ] {
            let before = probe.calls().len();
            field_with(
                &mut emu, &mut probe, receiver, class, name, signature, is_static,
            )?;
            assert_eq!(probe.calls().len(), before + 1);
        }
        let app = application(&mut emu)?;
        let before = probe.calls().len();
        member(
            &mut emu,
            &mut probe,
            app,
            "android/content/Context",
            "getPackageName(I)Ljava/lang/String;",
            &[0.into()],
            0,
        )?;
        member(
            &mut emu,
            &mut probe,
            Handle(0),
            "android/content/pm/Signature",
            "notAConstructor([B)V",
            &[Handle(0).into()],
            6,
        )?;
        assert_eq!(probe.calls().len(), before + 2);
    }
    Ok(())
}
