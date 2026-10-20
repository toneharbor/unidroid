use minidbg::{Emulator, Handle, JniCall, JniHandler, JniValue, Result};

struct JnaHandler;
impl JniHandler for JnaHandler {
    fn call_static_object_method(
        &mut self,
        emu: &mut Emulator,
        call: &JniCall<'_>,
    ) -> Option<Handle> {
        if call.class == "java/lang/System"
            && call.name == "getProperty"
            && call.signature == "(Ljava/lang/String;)Ljava/lang/String;"
            && emu.string(call.args[0].object()).ok()? == "file.encoding"
        {
            Some(emu.new_string("UTF-8"))
        } else {
            None
        }
    }
    fn get_static_object_field(
        &mut self,
        emu: &mut Emulator,
        call: &JniCall<'_>,
    ) -> Option<Handle> {
        if call.name != "TYPE" || call.signature != "Ljava/lang/Class;" {
            return None;
        }
        let name = match call.class {
            "java/lang/Void" => "void",
            "java/lang/Boolean" => "boolean",
            "java/lang/Byte" => "byte",
            "java/lang/Character" => "char",
            "java/lang/Short" => "short",
            "java/lang/Integer" => "int",
            "java/lang/Long" => "long",
            "java/lang/Float" => "float",
            "java/lang/Double" => "double",
            _ => return None,
        };
        Some(emu.class(name))
    }
    fn new_object(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<Handle> {
        if call.class == "java/lang/String" && call.name == "<init>" && call.signature == "([B)V" {
            Some(emu.new_string(&String::from_utf8_lossy(
                &emu.byte_array(call.args[0].object()).ok()?,
            )))
        } else {
            None
        }
    }
}

pub struct Output {
    pub sdk: String,
    pub malloc_nonzero: bool,
    pub version: String,
    pub checksum: String,
    pub size: i32,
}
pub fn run(emu: &mut Emulator, directory: &std::path::Path) -> Result<Output> {
    let module = emu.load_library(directory.join("libjnidispatch.so"))?;
    let mut handler = JnaHandler;
    emu.call_jni_onload(module, &mut handler)?;
    let property = emu.find_symbol(module, "__system_property_get")?;
    let key = emu.new_cstring("ro.build.version.sdk")?;
    let output = emu.malloc(16)?;
    let length = emu.call_function(property, &[key, output])?;
    let sdk = String::from_utf8(emu.read_bytes(output, length as usize)?)?;
    let class = "com/sun/jna/Native";
    let malloc_nonzero = emu
        .call_static_jni_method(&mut handler, class, "malloc(J)J", &[JniValue::Long(32)])?
        .long()
        != 0;
    let object = emu
        .call_static_jni_method(
            &mut handler,
            class,
            "getNativeVersion()Ljava/lang/String;",
            &[],
        )?
        .object();
    let version = emu.string(object)?;
    emu.release(object);
    let object = emu
        .call_static_jni_method(
            &mut handler,
            class,
            "getAPIChecksum()Ljava/lang/String;",
            &[],
        )?
        .object();
    let checksum = emu.string(object)?;
    emu.release(object);
    let size = emu
        .call_static_jni_method(&mut handler, class, "sizeof(I)I", &[0.into()])?
        .int();
    Ok(Output {
        sdk,
        malloc_nonzero,
        version,
        checksum,
        size,
    })
}
