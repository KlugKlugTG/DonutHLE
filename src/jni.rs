//! Minimal JNI environment for guest native code.
//!
//! Builds a `JNIEnv` whose function-table entries are host slots, so native
//! calls through `env->functions->X(...)` land in the host dispatcher with
//! the entry's name attached. Behavior lives in [`crate::host::BasicHost`];
//! unimplemented entries log their name and return 0, which is exactly the
//! measurement that drives the next bridge work.

use crate::arm::Machine;

/// Guest range holding the JNIEnv, its function table, and JNI objects
/// (strings, arrays) allocated for the guest.
pub const JNI_BASE: u32 = 0x7300_0000;
pub const JNI_SIZE: u32 = 0x0010_0000;

/// Function table starts after the JNIEnv word.
const VTABLE_OFFSET: u32 = 0x100;
/// JNI objects (strings, arrays) are bump-allocated from here.
pub const JNI_OBJECTS_OFFSET: u32 = 0x1000;

/// JNINativeInterface entries in vtable order. Guests hardcode the byte
/// offsets (index * 4), so this order must match the platform ABI exactly.
pub const JNI_TABLE: &[&str] = &[
    "reserved0",
    "reserved1",
    "reserved2",
    "reserved3",
    "GetVersion",
    "DefineClass",
    "FindClass",
    "FromReflectedMethod",
    "FromReflectedField",
    "ToReflectedMethod",
    "GetSuperclass",
    "IsAssignableFrom",
    "ToReflectedField",
    "Throw",
    "ThrowNew",
    "ExceptionOccurred",
    "ExceptionDescribe",
    "ExceptionClear",
    "FatalError",
    "PushLocalFrame",
    "PopLocalFrame",
    "NewGlobalRef",
    "DeleteGlobalRef",
    "DeleteLocalRef",
    "IsSameObject",
    "NewLocalRef",
    "EnsureLocalCapacity",
    "AllocObject",
    "NewObject",
    "NewObjectV",
    "NewObjectA",
    "GetObjectClass",
    "IsInstanceOf",
    "GetMethodID",
    "CallObjectMethod",
    "CallObjectMethodV",
    "CallObjectMethodA",
    "CallBooleanMethod",
    "CallBooleanMethodV",
    "CallBooleanMethodA",
    "CallByteMethod",
    "CallByteMethodV",
    "CallByteMethodA",
    "CallCharMethod",
    "CallCharMethodV",
    "CallCharMethodA",
    "CallShortMethod",
    "CallShortMethodV",
    "CallShortMethodA",
    "CallIntMethod",
    "CallIntMethodV",
    "CallIntMethodA",
    "CallLongMethod",
    "CallLongMethodV",
    "CallLongMethodA",
    "CallFloatMethod",
    "CallFloatMethodV",
    "CallFloatMethodA",
    "CallDoubleMethod",
    "CallDoubleMethodV",
    "CallDoubleMethodA",
    "CallVoidMethod",
    "CallVoidMethodV",
    "CallVoidMethodA",
    "CallNonvirtualObjectMethod",
    "CallNonvirtualObjectMethodV",
    "CallNonvirtualObjectMethodA",
    "CallNonvirtualBooleanMethod",
    "CallNonvirtualBooleanMethodV",
    "CallNonvirtualBooleanMethodA",
    "CallNonvirtualByteMethod",
    "CallNonvirtualByteMethodV",
    "CallNonvirtualByteMethodA",
    "CallNonvirtualCharMethod",
    "CallNonvirtualCharMethodV",
    "CallNonvirtualCharMethodA",
    "CallNonvirtualShortMethod",
    "CallNonvirtualShortMethodV",
    "CallNonvirtualShortMethodA",
    "CallNonvirtualIntMethod",
    "CallNonvirtualIntMethodV",
    "CallNonvirtualIntMethodA",
    "CallNonvirtualLongMethod",
    "CallNonvirtualLongMethodV",
    "CallNonvirtualLongMethodA",
    "CallNonvirtualFloatMethod",
    "CallNonvirtualFloatMethodV",
    "CallNonvirtualFloatMethodA",
    "CallNonvirtualDoubleMethod",
    "CallNonvirtualDoubleMethodV",
    "CallNonvirtualDoubleMethodA",
    "CallNonvirtualVoidMethod",
    "CallNonvirtualVoidMethodV",
    "CallNonvirtualVoidMethodA",
    "GetFieldID",
    "GetObjectField",
    "GetBooleanField",
    "GetByteField",
    "GetCharField",
    "GetShortField",
    "GetIntField",
    "GetLongField",
    "GetFloatField",
    "GetDoubleField",
    "SetObjectField",
    "SetBooleanField",
    "SetByteField",
    "SetCharField",
    "SetShortField",
    "SetIntField",
    "SetLongField",
    "SetFloatField",
    "SetDoubleField",
    "GetStaticMethodID",
    "CallStaticObjectMethod",
    "CallStaticObjectMethodV",
    "CallStaticObjectMethodA",
    "CallStaticBooleanMethod",
    "CallStaticBooleanMethodV",
    "CallStaticBooleanMethodA",
    "CallStaticByteMethod",
    "CallStaticByteMethodV",
    "CallStaticByteMethodA",
    "CallStaticCharMethod",
    "CallStaticCharMethodV",
    "CallStaticCharMethodA",
    "CallStaticShortMethod",
    "CallStaticShortMethodV",
    "CallStaticShortMethodA",
    "CallStaticIntMethod",
    "CallStaticIntMethodV",
    "CallStaticIntMethodA",
    "CallStaticLongMethod",
    "CallStaticLongMethodV",
    "CallStaticLongMethodA",
    "CallStaticFloatMethod",
    "CallStaticFloatMethodV",
    "CallStaticFloatMethodA",
    "CallStaticDoubleMethod",
    "CallStaticDoubleMethodV",
    "CallStaticDoubleMethodA",
    "CallStaticVoidMethod",
    "CallStaticVoidMethodV",
    "CallStaticVoidMethodA",
    "GetStaticFieldID",
    "GetStaticObjectField",
    "GetStaticBooleanField",
    "GetStaticByteField",
    "GetStaticCharField",
    "GetStaticShortField",
    "GetStaticIntField",
    "GetStaticLongField",
    "GetStaticFloatField",
    "GetStaticDoubleField",
    "SetStaticObjectField",
    "SetStaticBooleanField",
    "SetStaticByteField",
    "SetStaticCharField",
    "SetStaticShortField",
    "SetStaticIntField",
    "SetStaticLongField",
    "SetStaticFloatField",
    "SetStaticDoubleField",
    "NewString",
    "GetStringLength",
    "GetStringChars",
    "ReleaseStringChars",
    "NewStringUTF",
    "GetStringUTFLength",
    "GetStringUTFChars",
    "ReleaseStringUTFChars",
    "GetArrayLength",
    "NewObjectArray",
    "GetObjectArrayElement",
    "SetObjectArrayElement",
    "NewBooleanArray",
    "NewByteArray",
    "NewCharArray",
    "NewShortArray",
    "NewIntArray",
    "NewLongArray",
    "NewFloatArray",
    "NewDoubleArray",
    "GetBooleanArrayElements",
    "GetByteArrayElements",
    "GetCharArrayElements",
    "GetShortArrayElements",
    "GetIntArrayElements",
    "GetLongArrayElements",
    "GetFloatArrayElements",
    "GetDoubleArrayElements",
    "ReleaseBooleanArrayElements",
    "ReleaseByteArrayElements",
    "ReleaseCharArrayElements",
    "ReleaseShortArrayElements",
    "ReleaseIntArrayElements",
    "ReleaseLongArrayElements",
    "ReleaseFloatArrayElements",
    "ReleaseDoubleArrayElements",
    "GetBooleanArrayRegion",
    "GetByteArrayRegion",
    "GetCharArrayRegion",
    "GetShortArrayRegion",
    "GetIntArrayRegion",
    "GetLongArrayRegion",
    "GetFloatArrayRegion",
    "GetDoubleArrayRegion",
    "SetBooleanArrayRegion",
    "SetByteArrayRegion",
    "SetCharArrayRegion",
    "SetShortArrayRegion",
    "SetIntArrayRegion",
    "SetLongArrayRegion",
    "SetFloatArrayRegion",
    "SetDoubleArrayRegion",
    "RegisterNatives",
    "UnregisterNatives",
    "MonitorEnter",
    "MonitorExit",
    "GetJavaVM",
    "GetStringRegion",
    "GetStringUTFRegion",
    "GetPrimitiveArrayCritical",
    "ReleasePrimitiveArrayCritical",
    "GetStringCritical",
    "ReleaseStringCritical",
    "NewWeakGlobalRef",
    "DeleteWeakGlobalRef",
    "ExceptionCheck",
    "NewDirectByteBuffer",
    "GetDirectBufferAddress",
    "GetDirectBufferCapacity",
    "GetObjectRefType",
];

/// Host-slot name for a JNI table entry.
pub fn entry_name(index: usize) -> String {
    format!(
        "jni:{index}:{}",
        JNI_TABLE.get(index).copied().unwrap_or("unknown")
    )
}

/// Extracts the function name from a `jni:<index>:<Name>` host slot.
pub fn entry_function(slot_name: &str) -> &str {
    slot_name.rsplit(':').next().unwrap_or(slot_name)
}

/// Maps and binds the JNI environment; returns the `JNIEnv*` for the guest.
pub fn install(machine: &mut Machine) -> u32 {
    machine
        .memory
        .map_anon(JNI_BASE, JNI_SIZE)
        .expect("JNI region maps");
    let vtable = JNI_BASE + VTABLE_OFFSET;
    for index in 0..JNI_TABLE.len() {
        let name = entry_name(index);
        machine.linker.register_host(&name);
        let address = machine
            .linker
            .resolve(&name)
            .expect("JNI table entries are bound to host slots");
        machine
            .memory
            .write_u32(vtable + index as u32 * 4, address)
            .expect("JNI table writes");
    }
    machine
        .memory
        .write_u32(JNI_BASE, vtable)
        .expect("JNIEnv writes");
    JNI_BASE
}

/// Element sizes of the primitive array kinds, in bytes.
pub fn array_element_size(function: &str) -> Option<u32> {
    match function {
        "NewBooleanArray"
        | "GetBooleanArrayElements"
        | "ReleaseBooleanArrayElements"
        | "GetBooleanArrayRegion"
        | "SetBooleanArrayRegion" => Some(1),
        "NewByteArray"
        | "GetByteArrayElements"
        | "ReleaseByteArrayElements"
        | "GetByteArrayRegion"
        | "SetByteArrayRegion" => Some(1),
        "NewCharArray"
        | "GetCharArrayElements"
        | "ReleaseCharArrayElements"
        | "GetCharArrayRegion"
        | "SetCharArrayRegion" => Some(2),
        "NewShortArray"
        | "GetShortArrayElements"
        | "ReleaseShortArrayElements"
        | "GetShortArrayRegion"
        | "SetShortArrayRegion" => Some(2),
        "NewIntArray"
        | "GetIntArrayElements"
        | "ReleaseIntArrayElements"
        | "GetIntArrayRegion"
        | "SetIntArrayRegion" => Some(4),
        "NewLongArray"
        | "GetLongArrayElements"
        | "ReleaseLongArrayElements"
        | "GetLongArrayRegion"
        | "SetLongArrayRegion" => Some(8),
        "NewFloatArray"
        | "GetFloatArrayElements"
        | "ReleaseFloatArrayElements"
        | "GetFloatArrayRegion"
        | "SetFloatArrayRegion" => Some(4),
        "NewDoubleArray"
        | "GetDoubleArrayElements"
        | "ReleaseDoubleArrayElements"
        | "GetDoubleArrayRegion"
        | "SetDoubleArrayRegion" => Some(8),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arm::{CpuConfig, RETURN_SENTINEL};
    use crate::host::BasicHost;

    const CODE: u32 = 0x1000_0000;
    const STACK: u32 = 0x7F00_0000;

    #[test]
    fn vtable_offsets_match_the_platform_order() {
        // Byte offsets guests hardcode; these must never drift.
        assert_eq!(JNI_TABLE[6], "FindClass");
        assert_eq!(JNI_TABLE[31], "GetObjectClass");
        assert_eq!(JNI_TABLE[33], "GetMethodID");
        assert_eq!(JNI_TABLE[113], "GetStaticMethodID");
        assert_eq!(JNI_TABLE[141], "CallStaticVoidMethod");
        assert_eq!(JNI_TABLE[167], "NewStringUTF");
        assert_eq!(JNI_TABLE[169], "GetStringUTFChars");
        assert_eq!(JNI_TABLE[171], "GetArrayLength");
        assert_eq!(JNI_TABLE[179], "NewIntArray");
        assert_eq!(JNI_TABLE[215], "RegisterNatives");
        assert_eq!(JNI_TABLE[228], "ExceptionCheck");
    }

    #[test]
    fn guest_calls_find_class_through_the_vtable() {
        let mut machine = Machine::new(CpuConfig::default());
        machine.memory.map_anon(CODE, 0x1000).unwrap();
        machine.memory.map_anon(STACK, 0x10000).unwrap();
        machine.cpu.r[13] = STACK + 0xF000;
        let mut host = BasicHost::new();
        let env = install(&mut machine);

        // Guest sequence (word offsets):
        //   +00 ldr r0, [pc, #24]  ; r0 = env literal at +32
        //   +04 ldr r1, [r0]       ; r1 = functions
        //   +08 ldr r2, [r1, #24]  ; r2 = FindClass (6 * 4)
        //   +0C ldr r1, [pc, #16]  ; r1 = name literal at +36
        //   +10 blx r2             ; host returns to +14
        //   +14 ldr pc, [pc, #0]   ; jump through the word at +28
        //   +28 sentinel (0xFFFFFFFE)
        //   +32 env literal; +36 name literal
        let name_address = CODE + 0x200;
        machine
            .memory
            .write_cstr(name_address, "com/example/Game")
            .unwrap();
        let words = [
            0xE59F_0018u32, // ldr r0, [pc, #24]
            0xE590_1000,    // ldr r1, [r0]
            0xE591_2018,    // ldr r2, [r1, #24]
            0xE59F_1010,    // ldr r1, [pc, #16]
            0xE12F_FF32,    // blx r2
            0xE59F_F000,    // ldr pc, [pc, #0]
        ];
        for (index, word) in words.iter().enumerate() {
            machine
                .memory
                .write_u32(CODE + index as u32 * 4, *word)
                .unwrap();
        }
        machine
            .memory
            .write_u32(CODE + 28, RETURN_SENTINEL)
            .unwrap();
        machine.memory.write_u32(CODE + 32, env).unwrap();
        machine.memory.write_u32(CODE + 36, name_address).unwrap();
        assert_eq!(
            machine.memory.read_u32(CODE + 32).unwrap(),
            env,
            "env literal is stored"
        );

        let value = machine
            .call_function(&mut host, CODE, &[])
            .expect("guest JNI call runs");
        assert_ne!(value, 0, "FindClass returns a class handle");
        assert!(
            host.log
                .iter()
                .any(|line| line.contains("FindClass") && line.contains("com/example/Game")),
            "the call is visible in the log: {:?}",
            host.log
        );
    }
}
