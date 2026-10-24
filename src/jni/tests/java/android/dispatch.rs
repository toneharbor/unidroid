use super::*;

#[test]
fn activity_thread_and_package_manager_constants_answer_before_policy() -> Result<()> {
    for arch in ARCHES {
        let (file, _) = apk_emulator(arch)?;
        for process in [None, Some("custom.process")] {
            let mut builder = Emulator::builder().apk(&file.path);
            if let Some(process) = process {
                builder = builder.process_name(process);
            }
            let mut emu = builder.build()?;
            emu.setup_cpu(arch)?;
            let mut policy = crate::Policy::parse(
                r#"{"rules":[{"name":"currentPackageName","values":["fallback"]}]}"#,
            )?;
            let mut probe = crate::Probe::default();
            for (method, expected) in [
                ("currentPackageName()Ljava/lang/String;", "com.example.apk"),
                (
                    "currentProcessName()Ljava/lang/String;",
                    process.unwrap_or("com.example.apk"),
                ),
            ] {
                let mut handler = policy.handler(&mut probe);
                let value = member(
                    &mut emu,
                    &mut handler,
                    Handle(0),
                    "android/app/ActivityThread",
                    method,
                    &[],
                    1,
                )?
                .object();
                assert_eq!(emu.string(value)?, expected);
                handler.finish(Ok(()))?;
            }
            assert!(probe.calls().is_empty());
            for (name, expected) in [
                ("GET_SIGNATURES", 0x40),
                ("GET_SIGNING_CERTIFICATES", 0x08000000),
            ] {
                assert_eq!(
                    field(
                        &mut emu,
                        Handle(0),
                        "android/content/pm/PackageManager",
                        name,
                        "I",
                        true
                    )?,
                    JniValue::Int(expected)
                );
            }
            let app = application(&mut emu)?;
            let info = call(
                &mut emu,
                app,
                "getApplicationInfo()Landroid/content/pm/ApplicationInfo;",
                &[],
            )?
            .object();
            let value = field(
                &mut emu,
                info,
                "android/content/pm/ApplicationInfo",
                "processName",
                "Ljava/lang/String;",
                false,
            )?
            .object();
            assert_eq!(emu.string(value)?, process.unwrap_or("com.example.apk"));
            // An uncovered class consumes the untouched policy answer.
            let mut handler = policy.handler(&mut probe);
            let value = member(
                &mut emu,
                &mut handler,
                Handle(0),
                "app/Other",
                "currentPackageName()Ljava/lang/String;",
                &[],
                1,
            )?
            .object();
            assert_eq!(emu.string(value)?, "fallback");
            handler.finish(Ok(()))?;
        }
    }
    Ok(())
}

#[test]
fn android_hierarchy_and_declared_subclasses_work_through_jni_type_checks() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        emu.superclass("app/Child", "app/Parent");
        emu.superclass("app/Parent", "android/app/Activity");
        for (from, targets) in [
            (
                "android/app/Application",
                &[
                    "android/content/ContextWrapper",
                    "android/content/Context",
                    "java/lang/Object",
                ][..],
            ),
            (
                "android/app/Activity",
                &[
                    "android/view/ContextThemeWrapper",
                    "android/content/ContextWrapper",
                    "android/content/Context",
                ],
            ),
            (
                "app/Child",
                &[
                    "android/view/ContextThemeWrapper",
                    "android/content/Context",
                ],
            ),
            (
                "android/app/Service",
                &["android/content/ContextWrapper", "android/content/Context"],
            ),
            (
                "android/app/ApplicationPackageManager",
                &["android/content/pm/PackageManager", "java/lang/Object"],
            ),
            ("android/content/pm/PackageInfo", &["java/lang/Object"]),
            ("android/content/pm/ApplicationInfo", &["java/lang/Object"]),
            ("android/content/pm/Signature", &["java/lang/Object"]),
            ("android/content/pm/SigningInfo", &["java/lang/Object"]),
            ("android/content/res/AssetManager", &["java/lang/Object"]),
        ] {
            let object = emu.new_object(from, ());
            let source = emu.class(from);
            for target in targets {
                let target = emu.class(target);
                assert_eq!(jni_call(&mut emu, 32, &[object.0, target.0])?, 1, "{from}");
                assert_eq!(jni_call(&mut emu, 11, &[source.0, target.0])?, 1, "{from}");
            }
        }
        let app = application(&mut emu)?;
        let wrong = emu.class("android/content/pm/PackageInfo");
        assert_eq!(jni_call(&mut emu, 32, &[app.0, wrong.0])?, 0);
        let child = emu.new_object("app/Child", ());
        let package = call(&mut emu, child, "getPackageName()Ljava/lang/String;", &[])?.object();
        assert_eq!(emu.string(package)?, "com.example.apk");
    }
    Ok(())
}

#[test]
fn package_manager_leaves_other_packages_and_unknown_members_to_handlers() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        let app = application(&mut emu)?;
        let pm = call(
            &mut emu,
            app,
            "getPackageManager()Landroid/content/pm/PackageManager;",
            &[],
        )?
        .object();
        for method in [
            "getPackageInfo(Ljava/lang/String;I)Landroid/content/pm/PackageInfo;",
            "getApplicationInfo(Ljava/lang/String;I)Landroid/content/pm/ApplicationInfo;",
        ] {
            let other = emu.new_string("another.package");
            let mut probe = crate::Probe::default();
            assert_eq!(
                member(
                    &mut emu,
                    &mut probe,
                    pm,
                    "android/content/pm/PackageManager",
                    method,
                    &[other.into(), 0.into()],
                    0
                )?,
                Handle(0).into()
            );
            assert_eq!(probe.calls().len(), 1);
            let own = emu.new_string("com.example.apk");
            let result = member(
                &mut emu,
                &mut probe,
                pm,
                "android/content/pm/PackageManager",
                method,
                &[own.into(), 0.into()],
                0,
            )?
            .object();
            assert_ne!(result, Handle(0));
            assert_eq!(probe.calls().len(), 1);
            let error = call(&mut emu, pm, method, &[Handle(0).into(), 0.into()])
                .unwrap_err()
                .to_string();
            assert!(error.contains("java/lang/NullPointerException"), "{error}");
        }
        let mut probe = crate::Probe::default();
        for receiver in [app, pm] {
            member(
                &mut emu,
                &mut probe,
                receiver,
                "java/lang/Object",
                "uncovered()I",
                &[],
                0,
            )?;
            let hash = call(&mut emu, receiver, "hashCode()I", &[])?;
            assert_eq!(call(&mut emu, receiver, "hashCode()I", &[])?, hash);
            assert_eq!(
                call(
                    &mut emu,
                    receiver,
                    "equals(Ljava/lang/Object;)Z",
                    &[receiver.into()]
                )?,
                JniValue::Boolean(true)
            );
            assert!(text(&mut emu, receiver)?.contains('@'));
        }
        assert_eq!(probe.calls().len(), 1);
        assert_eq!(probe.calls()[0].count, 2);
        let opaque = emu.new_object("android/content/pm/PackageManager", ());
        let own = emu.new_string("com.example.apk");
        member(
            &mut emu,
            &mut probe,
            opaque,
            "android/content/pm/PackageManager",
            "getPackageInfo(Ljava/lang/String;I)Landroid/content/pm/PackageInfo;",
            &[own.into(), 0.into()],
            0,
        )?;
        assert_eq!(probe.calls().len(), 2);
    }
    Ok(())
}
