use super::*;

#[test]
fn application_identity_and_android_objects_rewind_with_live_metadata() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        let app = application(&mut emu)?;
        let app_hash = call(&mut emu, app, "hashCode()I", &[])?;
        let info = package_info(&mut emu, 0x40)?;
        emu.apk_mut().unwrap().signatures = vec![vec![1, 2, 255]];
        let array = package_field(
            &mut emu,
            info,
            "signatures",
            "[Landroid/content/pm/Signature;",
        )?
        .object();
        let signature = emu.object_array(array)?[0];
        let saved = emu.checkpoint()?;
        emu.release(app);
        emu.release(signature);
        emu.apk_mut().unwrap().package = "changed".into();
        emu.apk_mut().unwrap().signatures.clear();
        let mut fresh = Emulator::builder().build()?;
        for target in [&mut emu, &mut fresh] {
            target.rewind(&saved)?;
            let again = application(target)?;
            assert_ne!(again, app);
            assert_eq!(jni_call(target, 24, &[app.0, again.0])?, 1);
            assert_eq!(call(target, again, "hashCode()I", &[])?, app_hash);
            let context = call(
                target,
                app,
                "getApplicationContext()Landroid/content/Context;",
                &[],
            )?
            .object();
            assert_eq!(jni_call(target, 24, &[app.0, context.0])?, 1);
            assert_eq!(
                call(target, signature, "hashCode()I", &[])?,
                JniValue::Int(30813)
            );
            let bytes = call(target, signature, "toByteArray()[B", &[])?.object();
            assert_eq!(target.byte_array(bytes)?, [1, 2, 255]);
            let package =
                package_field(target, info, "packageName", "Ljava/lang/String;")?.object();
            assert_eq!(target.string(package)?, "com.example.apk");
            target.apk_mut().unwrap().version_code = 91;
            assert_eq!(
                call(target, info, "getLongVersionCode()J", &[])?,
                JniValue::Long(91)
            );
        }
        emu.rewind(&saved)?;
        assert_eq!(
            call(&mut emu, info, "getLongVersionCode()J", &[])?,
            JniValue::Long(0)
        );
        // The singleton survives even when no host/local handle owns it.
        emu.release(app);
        let again = application(&mut emu)?;
        assert_eq!(call(&mut emu, again, "hashCode()I", &[])?, app_hash);
    }
    Ok(())
}

#[test]
fn android_answers_return_fresh_local_references_and_native_manager_accepts_them() -> Result<()> {
    for arch in ARCHES {
        let (file, _) = apk_emulator(arch)?;
        let mut emu = Emulator::builder().apk(&file.path).build()?;
        let module = emu.load_library(format!("rootfs/{}/libc.so", arch.library_dir()))?;
        jni_call(&mut emu, 19, &[32])?;
        jni_call(&mut emu, 19, &[32])?;
        let app = application(&mut emu)?;
        let again = application(&mut emu)?;
        assert_ne!(app, again);
        assert_eq!(jni_call(&mut emu, 24, &[app.0, again.0])?, 1);
        for method in [
            "getPackageName()Ljava/lang/String;",
            "getPackageCodePath()Ljava/lang/String;",
            "getPackageResourcePath()Ljava/lang/String;",
            "getPackageManager()Landroid/content/pm/PackageManager;",
            "getApplicationInfo()Landroid/content/pm/ApplicationInfo;",
            "getAssets()Landroid/content/res/AssetManager;",
            "getApplicationContext()Landroid/content/Context;",
        ] {
            let first = call(&mut emu, app, method, &[])?.object();
            let second = call(&mut emu, app, method, &[])?.object();
            assert_ne!(first, second);
            for value in [first, second] {
                assert_eq!(jni_call(&mut emu, 232, &[value.0])?, 1);
            }
            jni_call(&mut emu, 23, &[first.0])?;
            assert_eq!(jni_call(&mut emu, 232, &[first.0])?, 0);
            assert_eq!(jni_call(&mut emu, 232, &[second.0])?, 1);
        }
        let manager = call(
            &mut emu,
            app,
            "getAssets()Landroid/content/res/AssetManager;",
            &[],
        )?
        .object();
        let function = emu.find_symbol(module, "AAssetManager_fromJava")?;
        let native = emu.call_function(function, &[emu.jni.env, manager.0])?;
        assert_ne!(native, 0);
        jni_call(&mut emu, 20, &[0])?;
        jni_call(&mut emu, 20, &[0])?;
        let manager = call(
            &mut emu,
            again,
            "getAssets()Landroid/content/res/AssetManager;",
            &[],
        );
        assert!(manager.is_err()); // Its local receiver expired with the frame.
        let app = application(&mut emu)?;
        let manager = call(
            &mut emu,
            app,
            "getAssets()Landroid/content/res/AssetManager;",
            &[],
        )?
        .object();
        assert_eq!(
            emu.call_function(function, &[emu.jni.env, manager.0])?,
            native
        );
    }
    Ok(())
}

#[test]
fn android_calls_fields_and_type_checks_fall_through_without_an_apk() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let mut probe = crate::Probe::default();
        emu.superclass("app/Host", "android/app/Application");
        let host = emu.new_object("app/Host", ());
        for method in [
            "getPackageName()Ljava/lang/String;",
            "getPackageCodePath()Ljava/lang/String;",
            "getPackageResourcePath()Ljava/lang/String;",
            "getPackageManager()Landroid/content/pm/PackageManager;",
            "getApplicationInfo()Landroid/content/pm/ApplicationInfo;",
            "getAssets()Landroid/content/res/AssetManager;",
            "getApplicationContext()Landroid/content/Context;",
        ] {
            assert_eq!(
                member(
                    &mut emu,
                    &mut probe,
                    host,
                    "android/content/Context",
                    method,
                    &[],
                    0
                )?,
                Handle(0).into()
            );
        }
        for method in [
            "currentApplication()Landroid/app/Application;",
            "currentPackageName()Ljava/lang/String;",
            "currentProcessName()Ljava/lang/String;",
        ] {
            assert_eq!(
                member(
                    &mut emu,
                    &mut probe,
                    Handle(0),
                    "android/app/ActivityThread",
                    method,
                    &[],
                    1
                )?,
                Handle(0).into()
            );
        }
        for name in ["GET_SIGNATURES", "GET_SIGNING_CERTIFICATES"] {
            assert_eq!(
                field_with(
                    &mut emu,
                    &mut probe,
                    Handle(0),
                    "android/content/pm/PackageManager",
                    name,
                    "I",
                    true
                )?,
                JniValue::Int(0)
            );
        }
        assert_eq!(probe.calls().len(), 12);
        for (class, method, args, op) in [
            (
                "android/content/pm/Signature",
                "<init>([B)V",
                vec![Handle(0).into()],
                6,
            ),
            ("android/content/pm/Signature", "toByteArray()[B", vec![], 0),
            (
                "android/content/pm/SigningInfo",
                "getApkContentsSigners()[Landroid/content/pm/Signature;",
                vec![],
                0,
            ),
            (
                "android/content/pm/PackageInfo",
                "getLongVersionCode()J",
                vec![],
                0,
            ),
            (
                "android/content/res/AssetManager",
                "list(Ljava/lang/String;)[Ljava/lang/String;",
                vec![Handle(0).into()],
                0,
            ),
        ] {
            let receiver = emu.new_object(class, ());
            let before = probe.calls().len();
            member(&mut emu, &mut probe, receiver, class, method, &args, op)?;
            assert_eq!(probe.calls().len(), before + 1);
        }
        let context = emu.class("android/content/Context");
        let table = emu.read_pointer(emu.jni.env)?;
        let function = emu.read_pointer(table + 32 * emu.pointer_size())?;
        assert_eq!(
            emu.call_function_with(&mut probe, function, &[emu.jni.env, host.0, context.0])?,
            0
        );
        assert_eq!(probe.calls().last().unwrap().op, "is_assignable");
    }
    Ok(())
}
