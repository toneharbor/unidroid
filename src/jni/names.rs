pub(super) fn jni_name(index: usize) -> String {
    const TYPES: [&str; 10] = [
        "Object", "Boolean", "Byte", "Char", "Short", "Int", "Long", "Float", "Double", "Void",
    ];
    const VARIANTS: [&str; 3] = ["", "V", "A"];
    match index {
        28..=30 => format!("NewObject{}", VARIANTS[index - 28]),
        34..=93 => {
            let base = if index < 64 { 34 } else { 64 };
            format!(
                "Call{}{}Method{}",
                if base == 64 { "Nonvirtual" } else { "" },
                TYPES[(index - base) / 3],
                VARIANTS[(index - base) % 3]
            )
        }
        114..=143 => format!(
            "CallStatic{}Method{}",
            TYPES[(index - 114) / 3],
            VARIANTS[(index - 114) % 3]
        ),
        95..=112 | 145..=162 => {
            let base = if index < 145 { 95 } else { 145 };
            let set = index - base >= 9;
            format!(
                "{}{}{}Field",
                if set { "Set" } else { "Get" },
                if base == 145 { "Static" } else { "" },
                TYPES[(index - base) % 9]
            )
        }
        175..=214 => {
            let block = (index - 175) / 8;
            format!(
                "{}{}Array{}",
                ["New", "Get", "Release", "Get", "Set"][block],
                TYPES[(index - 175) % 8 + 1],
                ["", "Elements", "Elements", "Region", "Region"][block]
            )
        }
        _ => match index {
            0..=3 => "reserved",
            4 => "GetVersion",
            5 => "DefineClass",
            6 => "FindClass",
            7 => "FromReflectedMethod",
            8 => "FromReflectedField",
            9 => "ToReflectedMethod",
            10 => "GetSuperclass",
            11 => "IsAssignableFrom",
            12 => "ToReflectedField",
            13 => "Throw",
            14 => "ThrowNew",
            15 => "ExceptionOccurred",
            16 => "ExceptionDescribe",
            17 => "ExceptionClear",
            18 => "FatalError",
            19 => "PushLocalFrame",
            20 => "PopLocalFrame",
            21 => "NewGlobalRef",
            22 => "DeleteGlobalRef",
            23 => "DeleteLocalRef",
            24 => "IsSameObject",
            25 => "NewLocalRef",
            26 => "EnsureLocalCapacity",
            27 => "AllocObject",
            31 => "GetObjectClass",
            32 => "IsInstanceOf",
            33 => "GetMethodID",
            94 => "GetFieldID",
            113 => "GetStaticMethodID",
            144 => "GetStaticFieldID",
            163 => "NewString",
            164 => "GetStringLength",
            165 => "GetStringChars",
            166 => "ReleaseStringChars",
            167 => "NewStringUTF",
            168 => "GetStringUTFLength",
            169 => "GetStringUTFChars",
            170 => "ReleaseStringUTFChars",
            171 => "GetArrayLength",
            172 => "NewObjectArray",
            173 => "GetObjectArrayElement",
            174 => "SetObjectArrayElement",
            215 => "RegisterNatives",
            216 => "UnregisterNatives",
            217 => "MonitorEnter",
            218 => "MonitorExit",
            219 => "GetJavaVM",
            220 => "GetStringRegion",
            221 => "GetStringUTFRegion",
            222 => "GetPrimitiveArrayCritical",
            223 => "ReleasePrimitiveArrayCritical",
            224 => "GetStringCritical",
            225 => "ReleaseStringCritical",
            226 => "NewWeakGlobalRef",
            227 => "DeleteWeakGlobalRef",
            228 => "ExceptionCheck",
            229 => "NewDirectByteBuffer",
            230 => "GetDirectBufferAddress",
            231 => "GetDirectBufferCapacity",
            232 => "GetObjectRefType",
            _ => unreachable!(),
        }
        .into(),
    }
}
pub(super) fn mangle(value: &str) -> String {
    let mut out = String::new();
    for c in value.encode_utf16() {
        match c {
            47 => out.push('_'),
            95 => out.push_str("_1"),
            59 => out.push_str("_2"),
            91 => out.push_str("_3"),
            48..=57 | 65..=90 | 97..=122 => out.push(char::from_u32(c as u32).unwrap()),
            _ => out.push_str(&format!("_0{c:04x}")),
        }
    }
    out
}
