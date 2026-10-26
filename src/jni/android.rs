use super::{Handle, JniCall, JniOp, JniValue, Object, java};
use crate::{Emulator, Result};
use java::string_utf8;
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

pub(super) fn dispatch(emu: &mut Emulator, call: &JniCall<'_>) -> Result<Option<JniValue>> {
    if call.op == JniOp::New
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
    if call.op == JniOp::Call && call.is_static && call.class == "android/app/ActivityThread" {
        return Ok(match (call.name, call.signature) {
            ("currentApplication", "()Landroid/app/Application;") => Some(application(emu)),
            ("currentPackageName", "()Ljava/lang/String;") => {
                Some(string_utf8(emu, &emu.apk().unwrap().package.clone()))
            }
            ("currentProcessName", "()Ljava/lang/String;") => {
                Some(string_utf8(emu, &emu.process_name.clone()))
            }
            _ => None,
        });
    }
    if call.op == JniOp::Get
        && call.is_static
        && call.class == "android/content/pm/PackageManager"
        && call.signature == "I"
    {
        return Ok(match call.name {
            "GET_SIGNATURES" => Some(JniValue::Int(0x40)),
            "GET_SIGNING_CERTIFICATES" => Some(JniValue::Int(0x08000000)),
            _ => None,
        });
    }
    if call.is_static || !matches!(call.op, JniOp::Call | JniOp::Get) {
        return Ok(None);
    }
    let receiver = emu.jni_object(call.receiver)?;
    if call.op == JniOp::Get {
        return packages::field(emu, call, &receiver.borrow());
    }
    if [receiver.borrow().java_type().as_str(), call.class]
        .iter()
        .any(|class| java::assignable(emu, class, "android/content/Context") == Some(true))
    {
        let value = match (call.name, call.signature) {
            ("getPackageName", "()Ljava/lang/String;") => {
                string_utf8(emu, &emu.apk().unwrap().package.clone())
            }
            ("getPackageCodePath" | "getPackageResourcePath", "()Ljava/lang/String;") => {
                string_utf8(emu, &emu.apk().unwrap().source_dir())
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
