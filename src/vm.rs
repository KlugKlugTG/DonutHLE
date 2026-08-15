use std::collections::HashMap;

use crate::dalvik::{CodeItem, DexFile};
use crate::framework::{Framework, FrameworkCall, FrameworkResult, Value as FrameworkValue};

pub type ObjectId = u32;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Void,
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Object(ObjectId),
    String(String),
    Null,
}

impl Eq for Value {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeapObject {
    Instance { class_name: String, fields: HashMap<u32, Value> },
    Array { component: String, values: Vec<Value> },
    String(String),
    Class(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmConfig { pub max_steps: usize, pub max_call_depth: usize }

impl Default for VmConfig { fn default() -> Self { Self { max_steps: 1_000_000, max_call_depth: 256 } } }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmError { pub pc: usize, pub opcode: u8, pub message: String }

impl std::fmt::Display for VmError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "Dalvik VM error at pc {} opcode 0x{:02x}: {}", self.pc, self.opcode, self.message) } }
impl std::error::Error for VmError {}

pub struct Vm<'a> {
    pub dex: &'a DexFile,
    pub framework: Framework,
    pub config: VmConfig,
    heap: Vec<HeapObject>,
    static_fields: HashMap<u32, Value>,
    call_depth: usize,
    executed_steps: usize,
}

impl<'a> Vm<'a> {
    pub fn new(dex: &'a DexFile, framework: Framework, config: VmConfig) -> Self { Self { dex, framework, config, heap: Vec::new(), static_fields: HashMap::new(), call_depth: 0, executed_steps: 0 } }
    pub fn heap_object(&self, id: ObjectId) -> Option<&HeapObject> { self.heap.get(id as usize) }
    pub fn alloc_instance(&mut self, class_name: impl Into<String>) -> ObjectId { self.alloc(HeapObject::Instance { class_name: class_name.into(), fields: HashMap::new() }) }
    pub fn alloc_string(&mut self, value: impl Into<String>) -> ObjectId { self.alloc(HeapObject::String(value.into())) }
    pub fn run_method(&mut self, method_index: usize, args: Vec<Value>) -> Result<Value, VmError> { self.call_method(method_index, args) }
    pub fn run_named_method(&mut self, class_name: &str, method_name: &str, args: Vec<Value>) -> Result<Value, VmError> {
        let method_index = self.dex.methods.iter().position(|method| method.class_name == class_name && method.name == method_name).ok_or_else(|| self.error(0, 0, format!("method {class_name}->{method_name} not found")))?;
        self.call_method(method_index, args)
    }
    fn call_method(&mut self, method_index: usize, args: Vec<Value>) -> Result<Value, VmError> {
        if self.call_depth >= self.config.max_call_depth { return Err(self.error(0, 0, "maximum call depth exceeded")); }
        let method = self.dex.method_id(method_index).ok_or_else(|| self.error(0, 0, format!("method index {method_index} is invalid")))?.clone();
        if method.class_name.starts_with("Landroid/") || method.class_name == "Ljava/lang/Class;" || method.class_name.starts_with("Ljava/lang/reflect/") { return self.dispatch_framework(&method.class_name, &method.name, &args); }
        let code = match self.dex.method_code_by_index(method_index) { Some(code) => code.clone(), None => { let flags = self.dex.method_access_flags(method_index).unwrap_or(0); if flags & (0x0100 | 0x0400) != 0 { return Ok(Value::Void); } return Err(self.error(0, 0, format!("method {} has no code (abstract/native methods are not executable)", method.name))); } };
        self.call_depth += 1;
        let result = self.execute_code(&code, args);
        self.call_depth -= 1;
        result
    }
    fn execute_code(&mut self, code: &CodeItem, args: Vec<Value>) -> Result<Value, VmError> {
        if code.registers_size < code.ins_size { return Err(self.error(0, 0, "register count is smaller than input count")); }
        let mut registers = vec![Value::Null; code.registers_size as usize];
        let first_input = code.registers_size as usize - code.ins_size as usize;
        for (index, value) in args.into_iter().enumerate() { let register = first_input + index; if register >= registers.len() { return Err(self.error(0, 0, "method argument exceeds input registers")); } registers[register] = value; }
        let mut pc = 0usize; let mut pending_result = Value::Void;
        while pc < code.instructions.len() {
            self.executed_steps += 1; if self.executed_steps > self.config.max_steps { return Err(self.error(pc, 0, "instruction limit exceeded")); }
            let instruction = code.instructions[pc]; let opcode = (instruction & 0xff) as u8;
            match opcode { 0x00 => pc += 1, _ => return Err(self.error(pc, opcode, "opcode is not implemented by the VM")) }
        }
        Ok(Value::Void)
    }
    fn dispatch_framework(&mut self, class_name: &str, method_name: &str, args: &[Value]) -> Result<Value, VmError> {
        if method_name == "<init>" || (class_name == "Landroid/app/Activity;" && matches!(method_name, "onCreate" | "onStart" | "onRestart" | "onResume" | "onPause" | "onStop" | "onDestroy" | "onNewIntent")) { return Ok(Value::Void); }
        if class_name == "Ljava/lang/Class;" && method_name == "forName" {
            let requested = string_arg(args, 0).map_err(|error| self.error(error.pc, error.opcode, "Class.forName expects a java.lang.String argument"))?;
            let descriptor = if requested.starts_with('L') && requested.ends_with(';') { requested } else { format!("L{};", requested.replace('.', "/")) };
            return Ok(Value::Object(self.alloc(HeapObject::Class(descriptor))));
        }
        let result = match (class_name, method_name) {
            ("Landroid/app/Activity;", "onCreate") | ("Landroid/app/Activity;", "onStart") | ("Landroid/app/Activity;", "onRestart") | ("Landroid/app/Activity;", "onResume") | ("Landroid/app/Activity;", "onPause") | ("Landroid/app/Activity;", "onStop") | ("Landroid/app/Activity;", "onDestroy") | ("Landroid/app/Activity;", "onSaveInstanceState") | ("Landroid/app/Activity;", "onRestoreInstanceState") | ("Landroid/app/Activity;", "onNewIntent") | ("Landroid/app/Activity;", "onWindowFocusChanged") => FrameworkResult::Void,
            _ => return Err(self.error(0, 0, format!("framework method {class_name}->{method_name} is not implemented")),
        };
        Ok(match result { FrameworkResult::Void => Value::Void, FrameworkResult::Int(v) => Value::Int(v), FrameworkResult::Bool(v) => Value::Int(i32::from(v)), FrameworkResult::Object(v) => if v == 0 { Value::Null } else { Value::Object(v) }, FrameworkResult::String(v) => Value::String(v) })
    }
    fn alloc(&mut self, object: HeapObject) -> ObjectId { let id = self.heap.len() as ObjectId; self.heap.push(object); id }
    fn error(&self, pc: usize, opcode: u8, message: impl Into<String>) -> VmError { VmError { pc, opcode, message: message.into() } }
}

fn get_register(registers: &[Value], index: usize, pc: usize, opcode: u8) -> Result<Value, VmError> { registers.get(index).cloned().ok_or_else(|| VmError { pc, opcode, message: format!("register v{index} is outside the frame") }) }
fn string_arg(args: &[Value], index: usize) -> Result<String, VmError> { match args.get(index) { Some(Value::String(value)) => Ok(value.clone()), Some(Value::Object(id)) => Err(VmError { pc: 0, opcode: 0, message: format!("framework argument {index} is object {id}, expected java.lang.String") }), _ => Err(VmError { pc: 0, opcode: 0, message: format!("framework argument {index} is not a string") }) } }
