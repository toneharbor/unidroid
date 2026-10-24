use super::*;

#[test]
fn package_flags_and_live_metadata_drive_package_and_application_info() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        let apk = emu.apk_mut().unwrap();
        apk.version_name = Some("1.2".into());
        apk.version_code = 0x1234_8000_0002;
        apk.signatures = vec![vec![1, 2, 255]];
        apk.signers = vec![vec![3, 4]];
        for flags in [0, 0x40, 0x08000000, 0x08000040] {
            let info = package_info(&mut emu, flags)?;
            for (name, expected) in [("packageName", "com.example.apk"), ("versionName", "1.2")] {
                let value = package_field(&mut emu, info, name, "Ljava/lang/String;")?.object();
                assert_eq!(emu.string(value)?, expected);
            }
            assert_eq!(
                package_field(&mut emu, info, "versionCode", "I")?,
                JniValue::Int(-2147483646)
            );
            assert_eq!(
                call(&mut emu, info, "getLongVersionCode()J", &[])?,
                JniValue::Long(0x1234_8000_0002)
            );
            let signatures = package_field(
                &mut emu,
                info,
                "signatures",
                "[Landroid/content/pm/Signature;",
            )?
            .object();
            assert_eq!(signatures.0 != 0, flags & 0x40 != 0);
            if signatures.0 != 0 {
                let entries = emu.object_array(signatures)?;
                assert_eq!(entries.len(), 1);
                let bytes = call(&mut emu, entries[0], "toByteArray()[B", &[])?.object();
                assert_eq!(emu.byte_array(bytes)?, [1, 2, 255]);
            }
            let signing = package_field(
                &mut emu,
                info,
                "signingInfo",
                "Landroid/content/pm/SigningInfo;",
            )?
            .object();
            assert_eq!(signing.0 != 0, flags & 0x08000000 != 0);
            let app = package_field(
                &mut emu,
                info,
                "applicationInfo",
                "Landroid/content/pm/ApplicationInfo;",
            )?
            .object();
            let lib = if arch == Architecture::Arm64 {
                "arm64"
            } else {
                "arm"
            };
            for (name, expected) in [
                ("packageName", "com.example.apk".to_owned()),
                ("sourceDir", "/data/app/com.example.apk-1/base.apk".into()),
                (
                    "publicSourceDir",
                    "/data/app/com.example.apk-1/base.apk".into(),
                ),
                (
                    "nativeLibraryDir",
                    format!("/data/app/com.example.apk-1/lib/{lib}"),
                ),
                ("dataDir", "/data/data/com.example.apk".into()),
                ("processName", "com.example.apk".into()),
            ] {
                let value = field(
                    &mut emu,
                    app,
                    "android/content/pm/ApplicationInfo",
                    name,
                    "Ljava/lang/String;",
                    false,
                )?
                .object();
                assert_eq!(emu.string(value)?, expected);
                if name == "sourceDir" {
                    assert_eq!(emu.file(&expected)?, std::fs::read(&_file.path)?);
                }
            }
        }
        let info = package_info(&mut emu, 0x08000040)?;
        let application_info = package_field(
            &mut emu,
            info,
            "applicationInfo",
            "Landroid/content/pm/ApplicationInfo;",
        )?
        .object();
        let app = application(&mut emu)?;
        let apk = emu.apk_mut().unwrap();
        apk.package = "changed.package".into();
        apk.version_name = None;
        apk.version_code = 7;
        apk.signatures = vec![vec![9]];
        assert_eq!(
            package_field(&mut emu, info, "versionName", "Ljava/lang/String;")?,
            Handle(0).into()
        );
        assert_eq!(
            package_field(&mut emu, info, "versionCode", "I")?,
            JniValue::Int(7)
        );
        assert_eq!(
            call(&mut emu, info, "getLongVersionCode()J", &[])?,
            JniValue::Long(7)
        );
        let value = package_field(&mut emu, info, "packageName", "Ljava/lang/String;")?.object();
        assert_eq!(emu.string(value)?, "changed.package");
        let array = package_field(
            &mut emu,
            info,
            "signatures",
            "[Landroid/content/pm/Signature;",
        )?
        .object();
        let sig = emu.object_array(array)?[0];
        let bytes = call(&mut emu, sig, "toByteArray()[B", &[])?.object();
        assert_eq!(emu.byte_array(bytes)?, [9]);
        for (name, expected) in [
            ("packageName", "changed.package"),
            ("sourceDir", "/data/app/changed.package-1/base.apk"),
            ("dataDir", "/data/data/changed.package"),
            ("processName", "com.example.apk"),
        ] {
            let value = field(
                &mut emu,
                application_info,
                "android/content/pm/ApplicationInfo",
                name,
                "Ljava/lang/String;",
                false,
            )?
            .object();
            assert_eq!(emu.string(value)?, expected);
        }
        let value = call(&mut emu, app, "getPackageName()Ljava/lang/String;", &[])?.object();
        assert_eq!(emu.string(value)?, "changed.package");
        let value = static_call(
            &mut emu,
            "android/app/ActivityThread",
            "currentPackageName()Ljava/lang/String;",
            &[],
        )?
        .object();
        assert_eq!(emu.string(value)?, "changed.package");
        let pm = call(
            &mut emu,
            app,
            "getPackageManager()Landroid/content/pm/PackageManager;",
            &[],
        )?
        .object();
        let name = emu.new_string("changed.package");
        let changed = call(
            &mut emu,
            pm,
            "getApplicationInfo(Ljava/lang/String;I)Landroid/content/pm/ApplicationInfo;",
            &[name.into(), 0.into()],
        )?
        .object();
        let value = field(
            &mut emu,
            changed,
            "android/content/pm/ApplicationInfo",
            "packageName",
            "Ljava/lang/String;",
            false,
        )?
        .object();
        assert_eq!(emu.string(value)?, "changed.package");
    }
    Ok(())
}
