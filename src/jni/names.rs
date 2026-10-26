use super::JniOp;

pub(super) struct MemberFunction {
    pub op: JniOp,
    pub is_static: bool,
    pub kind: usize,
    pub variant: usize,
    pub nonvirtual: bool,
    pub member_argument: usize,
}
impl MemberFunction {
    pub fn type_code(&self) -> u8 {
        match self.op {
            JniOp::Set => b'V',
            JniOp::New => b'L',
            _ => b"LZBCSIJFDV"[self.kind],
        }
    }
}

pub(super) fn member_function(index: usize) -> Option<MemberFunction> {
    use JniOp::*;
    let (op, is_static, base, methods, nonvirtual) = match index {
        28..=30 => (New, false, 28, true, false),
        34..=63 => (Call, false, 34, true, false),
        64..=93 => (Call, false, 64, true, true),
        95..=103 => (Get, false, 95, false, false),
        104..=112 => (Set, false, 104, false, false),
        114..=143 => (Call, true, 114, true, false),
        145..=153 => (Get, true, 145, false, false),
        154..=162 => (Set, true, 154, false, false),
        _ => return None,
    };
    let offset = index - base;
    Some(MemberFunction {
        op,
        is_static,
        kind: if methods { offset / 3 } else { offset },
        variant: if methods { offset % 3 } else { 0 },
        nonvirtual,
        member_argument: if nonvirtual { 3 } else { 2 },
    })
}

pub(super) fn jni_name(index: usize) -> String {
    const TYPES: [&str; 10] = [
        "Object", "Boolean", "Byte", "Char", "Short", "Int", "Long", "Float", "Double", "Void",
    ];
    const VARIANTS: [&str; 3] = ["", "V", "A"];
    if let Some(member) = member_function(index) {
        let prefix = if member.is_static {
            "Static"
        } else if member.nonvirtual {
            "Nonvirtual"
        } else {
            ""
        };
        return match member.op {
            JniOp::New => format!("NewObject{}", VARIANTS[member.variant]),
            JniOp::Call => format!(
                "Call{}{}Method{}",
                prefix, TYPES[member.kind], VARIANTS[member.variant]
            ),
            JniOp::Get | JniOp::Set => format!(
                "{}{}{}Field",
                if member.op == JniOp::Get {
                    "Get"
                } else {
                    "Set"
                },
                prefix,
                TYPES[member.kind]
            ),
        };
    }
    match index {
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
