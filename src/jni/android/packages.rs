use super::*;
use crate::PrimitiveType;

pub(super) fn signatures(emu: &mut Emulator, certificates: &[Vec<u8>]) -> JniValue {
    let values = certificates
        .iter()
        .map(|bytes| {
            Some(Rc::new(RefCell::new(Object::Android(Android::Signature(
                bytes.clone(),
            )))))
        })
        .collect();
    emu.insert_object(Object::ObjectArray(
        values,
        "android/content/pm/Signature".into(),
    ))
    .into()
}

pub(super) fn field(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    receiver: &Object,
) -> Result<Option<JniValue>> {
    let apk = emu.apk.as_ref().unwrap().clone();
    let value = match receiver {
        Object::Android(Android::PackageInfo(flags)) => match (call.name, call.signature) {
            ("packageName", "Ljava/lang/String;") => string_utf8(emu, &apk.package),
            ("versionName", "Ljava/lang/String;") => apk
                .version_name
                .as_ref()
                .map_or(Handle(0).into(), |value| string_utf8(emu, value)),
            ("versionCode", "I") => JniValue::Int(apk.version_code as i32),
            ("signatures", "[Landroid/content/pm/Signature;") => {
                if flags & 0x40 == 0 {
                    Handle(0).into()
                } else {
                    signatures(emu, &apk.signatures)
                }
            }
            ("signingInfo", "Landroid/content/pm/SigningInfo;") => {
                if flags & 0x08000000 == 0 {
                    Handle(0).into()
                } else {
                    object(emu, Android::SigningInfo)
                }
            }
            ("applicationInfo", "Landroid/content/pm/ApplicationInfo;") => {
                object(emu, Android::ApplicationInfo)
            }
            _ => return Ok(None),
        },
        Object::Android(Android::ApplicationInfo) if call.signature == "Ljava/lang/String;" => {
            let value = match call.name {
                "packageName" => apk.package.clone(),
                "sourceDir" | "publicSourceDir" => apk.source_dir(),
                "nativeLibraryDir" => apk.native_library_dir(emu.architecture()),
                "dataDir" => apk.data_dir(),
                "processName" => emu.process_name.clone(),
                _ => return Ok(None),
            };
            string_utf8(emu, &value)
        }
        _ => return Ok(None),
    };
    Ok(Some(value))
}

pub(super) fn call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    receiver: &Object,
) -> Result<Option<JniValue>> {
    let value = match (receiver, call.name, call.signature) {
        (Object::Android(Android::PackageManager), name, signature)
            if matches!(
                (name, signature),
                (
                    "getPackageInfo",
                    "(Ljava/lang/String;I)Landroid/content/pm/PackageInfo;"
                ) | (
                    "getApplicationInfo",
                    "(Ljava/lang/String;I)Landroid/content/pm/ApplicationInfo;"
                )
            ) =>
        {
            let name = String::from_utf16_lossy(&java::units(emu, call.args[0].object())?);
            if name != emu.apk().unwrap().package {
                return Ok(None);
            }
            object(
                emu,
                if call.name == "getPackageInfo" {
                    Android::PackageInfo(call.args[1].int())
                } else {
                    Android::ApplicationInfo
                },
            )
        }
        (Object::Android(Android::PackageInfo(_)), "getLongVersionCode", "()J") => {
            JniValue::Long(emu.apk().unwrap().version_code)
        }
        (Object::Android(Android::Signature(bytes)), "toCharsString", "()Ljava/lang/String;") => {
            let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            string_utf8(emu, &hex)
        }
        (Object::Android(Android::SigningInfo), name, signature) => {
            let apk = emu.apk.as_ref().unwrap().clone();
            match (name, signature) {
                ("getApkContentsSigners", "()[Landroid/content/pm/Signature;") => {
                    signatures(emu, &apk.signers)
                }
                ("hasMultipleSigners", "()Z") => JniValue::Boolean(apk.signers.len() > 1),
                ("hasPastSigningCertificates", "()Z") => {
                    JniValue::Boolean(!apk.signing_history.is_empty())
                }
                ("getSigningCertificateHistory", "()[Landroid/content/pm/Signature;") => {
                    if apk.signers.len() > 1 {
                        Handle(0).into()
                    } else if apk.signing_history.is_empty() {
                        signatures(emu, &apk.signers)
                    } else {
                        signatures(emu, &apk.signing_history)
                    }
                }
                _ => return Ok(None),
            }
        }
        (Object::Android(Android::Signature(bytes)), "toByteArray", "()[B") => emu
            .insert_object(Object::Array(PrimitiveType::Byte, bytes.clone()))
            .into(),
        _ => return Ok(None),
    };
    Ok(Some(value))
}
