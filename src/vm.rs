use std::collections::HashMap;
use std::fmt;

use crate::dalvik::{self, CodeItem, DexFile, ExecutionResult, InterpreterError, Registers};
use crate::framework::{Framework, HeapObject, Value as FrameworkValue};

pub type ObjectId = u32;
pub type Value = FrameworkValue;

#[derive(Debug, Clone, PartialEq)]
pub struct VmConfig {
    pub max_steps: usize,
    pub max_call_depth: usize,
    pub trace_registers: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmError {
    pub pc: usize,
    pub opcode: u8,
    pub method: String,
    pub message: String,
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Dalvik VM error at pc {} opcode {:#04x}: {} in {}", self.pc, self.opcode, self.message, self.method)
    }
}

impl std::error::Error for VmError {}

pub struct Vm<'dex> {
    pub dex: &'dex DexFile,
    pub framework: Framework,
    pub config: VmConfig,
    pub heap: Vec<HeapObject>,
    pub static_fields: HashMap<String, Value>,
    pub trace: Vec<String>,
}

impl<'dex> Vm<'dex> {
    pub fn new(dex: &'dex DexFile, framework: Framework, config: VmConfig) -> Self { unimplemented!() }
    pub fn alloc_instance(&mut self, class_name: String) -> ObjectId { unimplemented!() }
    pub fn run_method(&mut self, method_index: usize, args: Vec<Value>) -> Result<Value, VmError> { unimplemented!() }
    pub fn run_named_method(&mut self, class_name: &str, method_name: &str, args: Vec<Value>) -> Result<Value, VmError> { unimplemented!() }
    pub fn run_named_method_with_prototype(&mut self, class_name: &str, method_name: &str, prototype: Option<&str>, args: Vec<Value>) -> Result<Value, VmError> { unimplemented!() }
    pub fn run_instance_method(&mut self, object: ObjectId, method_name: &str, args: Vec<Value>) -> Result<Value, VmError> { unimplemented!() }
    pub fn render_frame(&mut self, object: ObjectId, method_name: &str) -> Result<Value, VmError> { unimplemented!() }
    pub fn find_instance_by_class(&self, class_name: &str) -> Option<ObjectId> { unimplemented!() }
    pub fn has_instance_method(&self, object: ObjectId, method_name: &str) -> bool { unimplemented!() }
    pub fn enable_register_trace(&mut self, enabled: bool) { self.config.trace_registers = enabled; }
    pub fn drain_trace(&mut self) -> Vec<String> { std::mem::take(&mut self.trace) }
}
