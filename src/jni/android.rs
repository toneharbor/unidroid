use super::{Handle, JniCall, JniValue, Object, java};
use crate::{Emulator, Result};
use std::{cell::RefCell, rc::Rc};

mod packages;

#[derive(Clone)]
pub(super) enum Android {
    Application,
    PackageManager,
    PackageInfo(i32),
    Signature(Vec<u8>),
    SigningInfo,
    ApplicationInfo,
    AssetManager,
}
impl Android {
    pub(super) fn class(&self) -> &'static str {
        match self {
            Self::Application => "android/app/Application",
            Self::PackageManager => "android/app/ApplicationPackageManager",
            Self::PackageInfo(_) => "android/content/pm/PackageInfo",
            Self::Signature(_) => "android/content/pm/Signature",
            Self::SigningInfo => "android/content/pm/SigningInfo",
            Self::ApplicationInfo => "android/content/pm/ApplicationInfo",
            Self::AssetManager => "android/content/res/AssetManager",
        }
    }
}

pub(super) fn parent(class: &str) -> Option<&'static str> {
    Some(match class {
        "android/app/Application" | "android/app/Service" | "android/view/ContextThemeWrapper" => {
            "android/content/ContextWrapper"
        }
        "android/app/Activity" => "android/view/ContextThemeWrapper",
        "android/content/ContextWrapper" => "android/content/Context",
        "android/app/ApplicationPackageManager" => "android/content/pm/PackageManager",
        "android/content/Context"
        | "android/content/pm/PackageManager"
        | "android/content/pm/PackageInfo"
        | "android/content/pm/Signature"
        | "android/content/pm/SigningInfo"
        | "android/content/pm/ApplicationInfo"
        | "android/content/res/AssetManager" => "java/lang/Object",
        _ => return None,
    })
}

fn string(emu: &mut Emulator, value: &str) -> JniValue {
    java::string(emu, value.encode_utf16().collect::<Vec<_>>())
}
fn object(emu: &mut Emulator, value: Android) -> JniValue {
    emu.insert_object(Object::Android(value)).into()
}
fn application(emu: &mut Emulator) -> JniValue {
    let application = emu
        .jni
        .application
        .get_or_insert_with(|| Rc::new(RefCell::new(Object::Android(Android::Application))))
        .clone();
    java::return_ref(emu, Some(application))
}

pub(super) fn dispatch(emu: &mut Emulator, call: &JniCall<'_>, op: u8) -> Result<Option<JniValue>> {
    if op == 6
        && call.class == "android/content/pm/Signature"
        && call.name == "<init>"
        && call.signature == "([B)V"
    {
        let input = call.args[0].object();
        if input.0 == 0 {
            return Err(java::exception("java/lang/NullPointerException", None));
        }
        let bytes = emu.byte_array(input)?;
        return Ok(Some(object(emu, Android::Signature(bytes))));
    }
    if op == 1 && call.class == "android/app/ActivityThread" {
        return Ok(match (call.name, call.signature) {
            ("currentApplication", "()Landroid/app/Application;") => Some(application(emu)),
            ("currentPackageName", "()Ljava/lang/String;") => {
                Some(string(emu, &emu.apk().unwrap().package.clone()))
            }
            ("currentProcessName", "()Ljava/lang/String;") => {
                Some(string(emu, &emu.process_name.clone()))
            }
            _ => None,
        });
    }
    if op == 3 && call.class == "android/content/pm/PackageManager" && call.signature == "I" {
        return Ok(match call.name {
            "GET_SIGNATURES" => Some(JniValue::Int(0x40)),
            "GET_SIGNING_CERTIFICATES" => Some(JniValue::Int(0x08000000)),
            _ => None,
        });
    }
    if !matches!(op, 0 | 2) {
        return Ok(None);
    }
    let receiver = emu.jni_object(call.receiver)?;
    if op == 2 {
        return packages::field(emu, call, &receiver.borrow());
    }
    if [receiver.borrow().java_type().as_str(), call.class]
        .iter()
        .any(|class| java::assignable(emu, class, "android/content/Context") == Some(true))
    {
        let value = match (call.name, call.signature) {
            ("getPackageName", "()Ljava/lang/String;") => {
                string(emu, &emu.apk().unwrap().package.clone())
            }
            ("getPackageCodePath" | "getPackageResourcePath", "()Ljava/lang/String;") => {
                string(emu, &emu.apk().unwrap().source_dir())
            }
            ("getPackageManager", "()Landroid/content/pm/PackageManager;") => {
                object(emu, Android::PackageManager)
            }
            ("getApplicationInfo", "()Landroid/content/pm/ApplicationInfo;") => {
                object(emu, Android::ApplicationInfo)
            }
            ("getAssets", "()Landroid/content/res/AssetManager;") => {
                object(emu, Android::AssetManager)
            }
            ("getApplicationContext", "()Landroid/content/Context;") => application(emu),
            _ => return Ok(None),
        };
        return Ok(Some(value));
    }
    if matches!(&*receiver.borrow(), Object::Android(Android::AssetManager))
        && call.name == "list"
        && call.signature == "(Ljava/lang/String;)[Ljava/lang/String;"
    {
        let path = String::from_utf16_lossy(&java::units(emu, call.args[0].object())?);
        let names = emu.apk().unwrap().asset_names(&path, false)?;
        let items = names
            .into_iter()
            .map(|name| {
                Some(Rc::new(RefCell::new(Object::String(
                    name.encode_utf16().collect(),
                ))))
            })
            .collect();
        return Ok(Some(
            emu.insert_object(Object::ObjectArray(items, "java/lang/String".into()))
                .into(),
        ));
    }
    packages::call(emu, call, &receiver.borrow())
}
