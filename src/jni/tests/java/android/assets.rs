use super::*;
use zip::CompressionMethod::{Deflated, Stored};

#[test]
fn asset_manager_lists_direct_files_and_folders_in_name_order() -> Result<()> {
    for arch in ARCHES {
        let file = TestApk::assets(&[
            ("z.txt", b"z", Stored),
            ("dir/b.txt", b"b", Deflated),
            ("dir/sub/deep.txt", b"deep", Stored),
            ("dir/a.txt", b"a", Stored),
            ("empty/", b"", Stored),
        ])?;
        let mut emu = Emulator::builder().apk(&file.path).build()?;
        emu.setup_cpu(arch)?;
        let app = application(&mut emu)?;
        let manager = call(
            &mut emu,
            app,
            "getAssets()Landroid/content/res/AssetManager;",
            &[],
        )?
        .object();
        for (path, expected) in [
            ("", vec!["dir", "empty", "z.txt"]),
            ("dir", vec!["a.txt", "b.txt", "sub"]),
            ("dir/sub", vec!["deep.txt"]),
            ("empty", vec![]),
            ("missing", vec![]),
        ] {
            let path = emu.new_string(path);
            let array = call(
                &mut emu,
                manager,
                "list(Ljava/lang/String;)[Ljava/lang/String;",
                &[path.into()],
            )?
            .object();
            let names = emu
                .object_array(array)?
                .into_iter()
                .map(|handle| emu.string(handle))
                .collect::<Result<Vec<_>>>()?;
            assert_eq!(names, expected);
        }
        let error = call(
            &mut emu,
            manager,
            "list(Ljava/lang/String;)[Ljava/lang/String;",
            &[Handle(0).into()],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("java/lang/NullPointerException"), "{error}");
    }
    Ok(())
}
