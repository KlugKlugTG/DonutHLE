use std::collections::HashMap;

use crate::dalvik::{CodeItem, DexFile};
use crate::framework::{Framework, FrameworkCall, FrameworkResult, Value as FrameworkValue};
use crate::Rgba8;

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
    Instance {
        class_name: String,
        fields: HashMap<String, Value>,
    },
    Array {
        component: String,
        values: Vec<Value>,
    },
    String(String),
    Class(String),
    Collection(Vec<Value>),
    StringBuilder(String),
    Boxed(Value),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmConfig {
    pub max_steps: usize,
    pub max_call_depth: usize,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            max_steps: 1_000_000,
            max_call_depth: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmError {
    pub pc: usize,
    pub opcode: u8,
    pub message: String,
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Dalvik VM error at pc {} opcode 0x{:02x}: {}",
            self.pc, self.opcode, self.message
        )
    }
}

impl std::error::Error for VmError {}

pub struct Vm<'a> {
    pub dex: &'a DexFile,
    pub framework: Framework,
    pub config: VmConfig,
    heap: Vec<HeapObject>,
    static_fields: HashMap<String, Value>,
    initialized_classes: std::collections::HashSet<String>,
    call_depth: usize,
    executed_steps: usize,
}

impl<'a> Vm<'a> {
    pub fn new(dex: &'a DexFile, framework: Framework, config: VmConfig) -> Self {
        Self {
            dex,
            framework,
            config,
            heap: Vec::new(),
            static_fields: HashMap::new(),
            initialized_classes: std::collections::HashSet::new(),
            call_depth: 0,
            executed_steps: 0,
        }
    }

    pub fn heap_object(&self, id: ObjectId) -> Option<&HeapObject> {
        self.heap.get(id as usize)
    }

    pub fn alloc_instance(&mut self, class_name: impl Into<String>) -> ObjectId {
        self.alloc(HeapObject::Instance {
            class_name: class_name.into(),
            fields: HashMap::new(),
        })
    }

    pub fn alloc_string(&mut self, value: impl Into<String>) -> ObjectId {
        self.alloc(HeapObject::String(value.into()))
    }

    pub fn alloc_collection(&mut self) -> ObjectId {
        self.alloc(HeapObject::Collection(Vec::new()))
    }

    pub fn run_method(&mut self, method_index: usize, args: Vec<Value>) -> Result<Value, VmError> {
        self.call_method(method_index, args)
    }

    pub fn run_named_method(
        &mut self,
        class_name: &str,
        method_name: &str,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        self.run_named_method_with_prototype(class_name, method_name, None, args)
    }

    pub fn run_named_method_with_prototype(
        &mut self,
        class_name: &str,
        method_name: &str,
        prototype: Option<&str>,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let method_index = self
            .dex
            .methods
            .iter()
            .position(|method| {
                method.class_name == class_name
                    && method.name == method_name
                    && prototype.is_none_or(|expected| method.prototype == expected)
            })
            .ok_or_else(|| {
                self.error(
                    0,
                    0,
                    format!("method {class_name}->{method_name} not found"),
                )
            })?;
        self.call_method(method_index, args)
    }

    pub fn run_instance_method(
        &mut self,
        object: ObjectId,
        method_name: &str,
        mut args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let class_name = match self.heap_object(object) {
            Some(HeapObject::Instance { class_name, .. }) => class_name.clone(),
            _ => return Err(self.error(0, 0, "listener is not an object instance")),
        };
        let method_index = self
            .dex
            .methods
            .iter()
            .enumerate()
            .find(|(index, method)| {
                method.class_name == class_name
                    && method.name == method_name
                    && self.dex.method_code_by_index(*index).is_some()
            })
            .map(|(index, _)| index)
            .or_else(|| {
                self.dex
                    .methods
                    .iter()
                    .enumerate()
                    .find(|(_, method)| {
                        method.class_name == class_name && method.name == method_name
                    })
                    .map(|(index, _)| index)
            })
            .ok_or_else(|| {
                self.error(
                    0,
                    0,
                    format!("method {class_name}->{method_name} not found"),
                )
            })?;
        args.insert(0, Value::Object(object));
        self.call_method(method_index, args)
    }

    pub fn find_instance_by_class(&self, class_name: &str) -> Option<ObjectId> {
        self.heap
            .iter()
            .enumerate()
            .find_map(|(id, value)| match value {
                HeapObject::Instance {
                    class_name: value_class,
                    ..
                } if value_class == class_name => Some(id as ObjectId),
                _ => None,
            })
    }

    fn instance_method_index(&self, object: ObjectId, referenced_index: usize) -> Option<usize> {
        let referenced = self.dex.method_id(referenced_index)?;
        let class_name = match self.heap_object(object)? {
            HeapObject::Instance { class_name, .. } => class_name,
            _ => return None,
        };
        let mut current = class_name.as_str();
        let mut visited = std::collections::BTreeSet::new();
        while visited.insert(current.to_owned()) {
            if let Some((index, _)) = self.dex.methods.iter().enumerate().find(|(index, method)| {
                method.class_name == current
                    && method.name == referenced.name
                    && method.prototype == referenced.prototype
                    && self.dex.method_code_by_index(*index).is_some()
            }) {
                return Some(index);
            }
            current = self.dex.find_class(current)?.super_class.as_deref()?;
        }
        None
    }

    fn ensure_class_initialized(&mut self, class_name: &str) -> Result<(), VmError> {
        if !self.initialized_classes.insert(class_name.to_owned()) {
            return Ok(());
        }
        if let Some((initializer_index, _)) =
            self.dex.methods.iter().enumerate().find(|(_, candidate)| {
                candidate.class_name == class_name && candidate.name == "<clinit>"
            })
        {
            self.call_method(initializer_index, Vec::new())?;
        }
        Ok(())
    }

    fn call_method(&mut self, method_index: usize, args: Vec<Value>) -> Result<Value, VmError> {
        if self.call_depth >= self.config.max_call_depth {
            return Err(self.error(0, 0, "maximum call depth exceeded"));
        }
        let method = self
            .dex
            .method_id(method_index)
            .ok_or_else(|| self.error(0, 0, format!("method index {method_index} is invalid")))?
            .clone();
        if method.name != "<clinit>" {
            self.ensure_class_initialized(&method.class_name)?;
        }
        let framework_class = method.class_name.starts_with("Landroid/")
            || method.class_name.starts_with("Ljava/")
            || method.class_name.starts_with("Ldalvik/")
            || method.class_name.starts_with("Lcom/badlogic/gdx/")
            || method.class_name.starts_with("Lcom/hyperkani/common/");
        if framework_class {
            if method.name == "<clinit>" {
                return Ok(Value::Void);
            }
            if method.class_name.starts_with("Lcom/badlogic/gdx/") {
                if method.name == "<init>" || method.name == "<clinit>" {
                    return Ok(Value::Void);
                }
                return self.dispatch_gdx(&method.class_name, &method.name, &args);
            }
            if let Some(code) = self.dex.method_code_by_index(method_index).cloned() {
                self.call_depth += 1;
                let result = self.execute_code(&code, args);
                self.call_depth -= 1;
                return result;
            }
            if let Some(owner) = self
                .dex
                .framework_method_owner(&method.class_name, &method.name)
            {
                return self
                    .dispatch_framework(&owner, &method.name, &args)
                    .map_err(|error| {
                        self.error(
                            error.pc,
                            error.opcode,
                            format!(
                                "{} while dispatching {}->{}",
                                error.message, owner, method.name
                            ),
                        )
                    });
            }
            return self
                .dispatch_framework(&method.class_name, &method.name, &args)
                .map_err(|error| {
                    self.error(
                        error.pc,
                        error.opcode,
                        format!(
                            "{} while dispatching {}->{}",
                            error.message, method.class_name, method.name
                        ),
                    )
                });
        }
        let code = match self.dex.method_code_by_index(method_index) {
            Some(code) => code.clone(),
            None => {
                if let Some(owner) = self
                    .dex
                    .framework_method_owner(&method.class_name, &method.name)
                {
                    return self.dispatch_framework(&owner, &method.name, &args);
                }
                let flags = self.dex.method_access_flags(method_index).unwrap_or(0);
                if flags & (0x0100 | 0x0400) != 0 {
                    return Ok(Value::Void);
                }
                return Err(self.error(
                    0,
                    0,
                    format!(
                        "method {} has no code (abstract/native methods are not executable)",
                        method.name
                    ),
                ));
            }
        };
        self.call_depth += 1;
        let result = self.execute_code(&code, args);
        self.call_depth -= 1;
        result.map_err(|mut error| {
            error.message = format!(
                "{} in {}->{}",
                error.message, method.class_name, method.name
            );
            error
        })
    }

    fn execute_code(&mut self, code: &CodeItem, args: Vec<Value>) -> Result<Value, VmError> {
        if code.registers_size < code.ins_size {
            return Err(self.error(0, 0, "register count is smaller than input count"));
        }
        let mut registers = vec![Value::Null; code.registers_size as usize];
        let first_input = code.registers_size as usize - code.ins_size as usize;
        for (index, value) in args.into_iter().enumerate() {
            let register = first_input + index;
            if register >= registers.len() {
                return Err(self.error(0, 0, "method argument exceeds input registers"));
            }
            registers[register] = value;
        }
        let mut pc = 0usize;
        let mut pending_result = Value::Void;
        while pc < code.instructions.len() {
            self.executed_steps += 1;
            if self.executed_steps > self.config.max_steps {
                return Err(self.error(pc, 0, "instruction limit exceeded"));
            }
            let instruction = code.instructions[pc];
            let opcode = (instruction & 0xff) as u8;
            match opcode {
                0x00 => pc += 1,
                0x01 | 0x04 | 0x07 => {
                    let (dest, source) = two_registers(instruction);
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 1;
                }
                0x02 | 0x05 | 0x08 => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    let source = code_word(code, pc + 1, pc, opcode)? as usize;
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 2;
                }
                0x03 | 0x06 | 0x09 => {
                    let dest = code_word(code, pc + 1, pc, opcode)? as usize;
                    let source = code_word(code, pc + 2, pc, opcode)? as usize;
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 3;
                }
                0x0a..=0x0c => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    set_register(
                        &mut registers,
                        dest,
                        pending_result.clone(),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 1;
                }
                0x0d => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    let value = match pending_result.clone() {
                        Value::Long(value) => Value::Int((value >> 32) as i32),
                        Value::Double(value) => Value::Int(value.to_bits() as u32 as i32),
                        value => value,
                    };
                    set_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 1;
                }
                0x0e => return Ok(Value::Void),
                0x0f => return Ok(pending_result.clone()),
                0x10 => {
                    let value = match pending_result.clone() {
                        Value::Long(value) => Value::Int(value as i32),
                        Value::Double(value) => Value::Int((value.to_bits() >> 32) as i32),
                        value => value,
                    };
                    return Ok(value);
                }
                0x11 => return Ok(pending_result.clone()),
                0x12 => {
                    let register = ((instruction >> 8) & 0x0f) as usize;
                    let literal = ((instruction >> 12) & 0x0f) as i8 as i32;
                    set_register(
                        &mut registers,
                        register,
                        Value::Int(literal),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 1;
                }
                0x13 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let literal = code_word(code, pc + 1, pc, opcode)? as i16 as i32;
                    set_register(
                        &mut registers,
                        register,
                        Value::Int(literal),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 2;
                }
                0x14 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let low = code_word(code, pc + 1, pc, opcode)? as u32;
                    let high = code_word(code, pc + 2, pc, opcode)? as u32;
                    set_register(
                        &mut registers,
                        register,
                        Value::Int((low | high << 16) as i32),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 3;
                }
                0x15 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let value = (code_word(code, pc + 1, pc, opcode)? as i16 as i32) << 16;
                    set_register(
                        &mut registers,
                        register,
                        Value::Int(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 2;
                }
                0x16 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let value = code_word(code, pc + 1, pc, opcode)? as i16 as i64;
                    set_register(
                        &mut registers,
                        register,
                        Value::Long(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 2;
                }
                0x17 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let low = code_word(code, pc + 1, pc, opcode)? as u32;
                    let high = code_word(code, pc + 2, pc, opcode)? as u32;
                    set_register(
                        &mut registers,
                        register,
                        Value::Long((low | high << 16) as i64),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 3;
                }
                0x18 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let low = code_word(code, pc + 1, pc, opcode)? as u64;
                    let high = code_word(code, pc + 2, pc, opcode)? as u64;
                    let upper = code_word(code, pc + 3, pc, opcode)? as u64;
                    let top = code_word(code, pc + 4, pc, opcode)? as u64;
                    set_register(
                        &mut registers,
                        register,
                        Value::Long((low | high << 16 | upper << 32 | top << 48) as i64),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 5;
                }
                0x19 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let value = (code_word(code, pc + 1, pc, opcode)? as i16 as i64) << 48;
                    set_register(
                        &mut registers,
                        register,
                        Value::Long(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 2;
                }
                0x1a | 0x1b => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let index = if opcode == 0x1a {
                        code_word(code, pc + 1, pc, opcode)? as usize
                    } else {
                        let low = code_word(code, pc + 1, pc, opcode)? as u32;
                        let high = code_word(code, pc + 2, pc, opcode)? as u32;
                        (low | high << 16) as usize
                    };
                    let value = self
                        .dex
                        .strings
                        .get(index)
                        .cloned()
                        .ok_or_else(|| self.error(pc, opcode, "string index is invalid"))?;
                    set_register(
                        &mut registers,
                        register,
                        Value::String(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += if opcode == 0x1a { 2 } else { 3 };
                }
                0x1c => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let type_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let value = self
                        .dex
                        .types
                        .get(type_index)
                        .cloned()
                        .ok_or_else(|| self.error(pc, opcode, "class index is invalid"))?;
                    set_register(
                        &mut registers,
                        register,
                        Value::Object(self.alloc(HeapObject::Class(value))),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 2;
                }
                0x1d | 0x1e => {
                    pc += 1;
                }
                0x1f => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let type_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let class_name = self
                        .dex
                        .types
                        .get(type_index)
                        .cloned()
                        .ok_or_else(|| self.error(pc, opcode, "class type index is invalid"))?;
                    let object = self.alloc(HeapObject::Class(class_name));
                    set_register(
                        &mut registers,
                        register,
                        Value::Object(object),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 2;
                }
                0x20 => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    let source = ((instruction >> 12) & 0x0f) as usize;
                    let result = match get_register(&registers, source, pc, opcode)? {
                        Value::Object(id) => {
                            matches!(self.heap_object(id), Some(HeapObject::Instance { .. }))
                        }
                        Value::Null => false,
                        _ => false,
                    };
                    set_register(
                        &mut registers,
                        dest,
                        Value::Int(i32::from(result)),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 1;
                }
                0x21 => {
                    let dest = ((instruction >> 8) & 0x0f) as usize;
                    let array_register = ((instruction >> 12) & 0x0f) as usize;
                    let array = get_object(&registers, array_register, self, pc, opcode)?;
                    let length = match self.heap_object(array) {
                        Some(HeapObject::Array { values, .. }) => values.len() as i32,
                        _ => {
                            return Err(self.error(
                                pc,
                                opcode,
                                "array-length target is not an array",
                            ))
                        }
                    };
                    set_register(&mut registers, dest, Value::Int(length), self, pc, opcode)?;
                    pc += 1;
                }
                0x22 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let type_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let class_name = self.dex.types.get(type_index).cloned().ok_or_else(|| {
                        self.error(pc, opcode, "new-instance type index is invalid")
                    })?;
                    let object = if matches!(
                        class_name.as_str(),
                        "Ljava/util/ArrayList;" | "Ljava/util/LinkedList;" | "Ljava/util/HashMap;"
                    ) {
                        self.alloc(HeapObject::Collection(Vec::new()))
                    } else {
                        self.alloc_instance(class_name.clone())
                    };
                    if class_name.starts_with("Landroid/view/")
                        || class_name.starts_with("Landroid/widget/")
                    {
                        self.framework.ensure_view(object, class_name);
                    }
                    set_register(
                        &mut registers,
                        register,
                        Value::Object(object),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 2;
                }
                0x23 => {
                    let dest = ((instruction >> 8) & 0x0f) as usize;
                    let size_register = ((instruction >> 12) & 0x0f) as usize;
                    let size = as_int(
                        get_register(&registers, size_register, pc, opcode)?,
                        pc,
                        opcode,
                    )?;
                    if size < 0 {
                        return Err(self.error(pc, opcode, "negative array size"));
                    }
                    let type_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let component = self
                        .dex
                        .types
                        .get(type_index)
                        .cloned()
                        .ok_or_else(|| self.error(pc, opcode, "array type index is invalid"))?;
                    let object = self.alloc(HeapObject::Array {
                        component,
                        values: vec![Value::Null; size as usize],
                    });
                    set_register(
                        &mut registers,
                        dest,
                        Value::Object(object),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 2;
                }
                0x26 => {
                    let array_register = ((instruction >> 8) & 0xff) as usize;
                    let array = get_object(&registers, array_register, self, pc, opcode)?;
                    let offset = (code_word(code, pc + 1, pc, opcode)? as u32)
                        | ((code_word(code, pc + 2, pc, opcode)? as u32) << 16);
                    let payload = (pc as i64 + offset as i32 as i64) as usize;
                    if payload + 4 > code.instructions.len() || code.instructions[payload] != 0x0300
                    {
                        return Err(self.error(pc, opcode, "invalid fill-array-data payload"));
                    }
                    let element_width = code.instructions[payload + 1] as usize;
                    let size = (code.instructions[payload + 2] as u32)
                        | ((code.instructions[payload + 3] as u32) << 16);
                    let size = size as usize;
                    let words = (element_width * size).div_ceil(2);
                    if payload + 4 + words > code.instructions.len() {
                        return Err(self.error(pc, opcode, "truncated fill-array-data payload"));
                    }
                    let component = match self.heap_object(array) {
                        Some(HeapObject::Array { component, .. }) => component.clone(),
                        _ => {
                            return Err(self.error(
                                pc,
                                opcode,
                                "fill-array-data target is not an array",
                            ))
                        }
                    };
                    let mut values = Vec::with_capacity(size);
                    for index in 0..size {
                        let bit_offset = index * element_width;
                        let mut raw = 0u32;
                        for byte in 0..element_width {
                            let unit = code.instructions[payload + 4 + (bit_offset + byte) / 2];
                            let value = if (bit_offset + byte).is_multiple_of(2) {
                                (unit & 0xff) as u32
                            } else {
                                (unit >> 8) as u32
                            };
                            raw |= value << (byte * 8);
                        }
                        values.push(if component == "F" {
                            Value::Float(f32::from_bits(raw))
                        } else {
                            Value::Int(raw as i32)
                        });
                    }
                    match self.heap.get_mut(array as usize) {
                        Some(HeapObject::Array { values: target, .. }) => *target = values,
                        _ => {
                            return Err(self.error(
                                pc,
                                opcode,
                                "fill-array-data target is not an array",
                            ))
                        }
                    }
                    pc += 3;
                }
                0x27 => {
                    return Err(self.error(pc, opcode, "throw is not implemented"));
                }
                0x28 => pc = pc.saturating_add(1),
                0x29 => {
                    let offset = code_word(code, pc + 1, pc, opcode)? as i8 as i32;
                    pc = branch_target(pc, offset, code.instructions.len(), pc, opcode)?;
                }
                0x2a => {
                    let offset = code_word(code, pc + 1, pc, opcode)? as i16 as i32;
                    pc = branch_target(pc, offset, code.instructions.len(), pc, opcode)?;
                }
                0x2d..=0x31 => {
                    let (dest, left_reg, right_reg) =
                        three_registers(instruction, code_word(code, pc + 1, pc, opcode)?);
                    let left =
                        as_float(get_register(&registers, left_reg, pc, opcode)?, pc, opcode)?;
                    let right =
                        as_float(get_register(&registers, right_reg, pc, opcode)?, pc, opcode)?;
                    let value = match opcode {
                        0x2d => left + right,
                        0x2e => left - right,
                        0x2f => left * right,
                        0x30 => left / right,
                        _ => left % right,
                    };
                    set_register(&mut registers, dest, Value::Float(value), self, pc, opcode)?;
                    pc += 2;
                }
                0x32..=0x37 => {
                    let (left, right) = two_registers(instruction);
                    let offset = code_word(code, pc + 1, pc, opcode)? as i16 as i32;
                    let left_value = get_register(&registers, left, pc, opcode)?;
                    let right_value = get_register(&registers, right, pc, opcode)?;
                    let equal = values_equal(&left_value, &right_value);
                    let take = if opcode <= 0x33 { equal } else { !equal };
                    if take {
                        pc = branch_target(pc, offset, code.instructions.len(), pc, opcode)?;
                    } else {
                        pc += 2;
                    }
                }
                0x38..=0x3d => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let offset = code_word(code, pc + 1, pc, opcode)? as i16 as i32;
                    let value = get_register(&registers, register, pc, opcode)?;
                    let zero = matches!(value, Value::Null)
                        || as_int(value.clone(), pc, opcode).unwrap_or(0) == 0;
                    let take = match opcode {
                        0x38 => zero,
                        0x39 => !zero,
                        0x3a => zero,
                        0x3b => !zero,
                        0x3c => zero,
                        _ => !zero,
                    };
                    if take {
                        pc = branch_target(pc, offset, code.instructions.len(), pc, opcode)?;
                    } else {
                        pc += 2;
                    }
                }
                0x44..=0x4b => {
                    let (dest, array_reg, index_reg) =
                        three_registers(instruction, code_word(code, pc + 1, pc, opcode)?);
                    let array = get_object(&registers, array_reg, self, pc, opcode)?;
                    let index =
                        as_int(get_register(&registers, index_reg, pc, opcode)?, pc, opcode)?
                            as usize;
                    let value = match self.heap_object(array) {
                        Some(HeapObject::Array { values, .. }) => values
                            .get(index)
                            .cloned()
                            .ok_or_else(|| self.error(pc, opcode, "array index out of bounds"))?,
                        _ => return Err(self.error(pc, opcode, "array target is not an array")),
                    };
                    set_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 2;
                }
                0x4c..=0x51 => {
                    let (value_reg, array_reg, index_reg) =
                        three_registers(instruction, code_word(code, pc + 1, pc, opcode)?);
                    let array = get_object(&registers, array_reg, self, pc, opcode)?;
                    let index =
                        as_int(get_register(&registers, index_reg, pc, opcode)?, pc, opcode)?
                            as usize;
                    let value = get_register(&registers, value_reg, pc, opcode)?.clone();
                    let array_len = match self.heap_object(array) {
                        Some(HeapObject::Array { values, .. }) => values.len(),
                        _ => 0,
                    };
                    if index >= array_len {
                        return Err(self.error(pc, opcode, "array index out of bounds"));
                    }
                    match self.heap.get_mut(array as usize) {
                        Some(HeapObject::Array { values, .. }) => values[index] = value,
                        _ => return Err(self.error(pc, opcode, "array target is not an array")),
                    }
                    pc += 2;
                }
                0x52..=0x5f => {
                    let (value_register, object_register) = two_registers(instruction);
                    let field_index = code_word(code, pc + 1, pc, opcode)? as u32;
                    let field_key = self
                        .dex
                        .field_key(field_index as usize)
                        .unwrap_or_else(|| format!("#{field_index}"));
                    if opcode <= 0x58 {
                        let object = get_object(&registers, object_register, self, pc, opcode)?;
                        let existing = match self.heap_object(object) {
                            Some(HeapObject::Instance { fields, .. }) => {
                                fields.get(&field_key).cloned()
                            }
                            _ => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    "instance field target is not an object",
                                ))
                            }
                        };
                        let value = if let Some(value) = existing {
                            value
                        } else if self
                            .dex
                            .field_id(field_index as usize)
                            .is_some_and(|field| field.name == "mMainLayer")
                        {
                            let layer = self.alloc_instance("Lcom/hyperkani/common/Layer;");
                            if let Some(HeapObject::Instance { fields, .. }) =
                                self.heap.get_mut(object as usize)
                            {
                                fields.insert(field_key, Value::Object(layer));
                            }
                            Value::Object(layer)
                        } else {
                            self.dex
                                .field_id(field_index as usize)
                                .map_or(Value::Null, |field| match field.type_name.as_str() {
                                    "I" | "Z" | "B" | "S" | "C" => Value::Int(0),
                                    "F" => Value::Float(0.0),
                                    "J" => Value::Long(0),
                                    "D" => Value::Double(0.0),
                                    _ => Value::Null,
                                })
                        };
                        set_register(&mut registers, value_register, value, self, pc, opcode)?;
                    } else {
                        let object = get_object(&registers, object_register, self, pc, opcode)?;
                        let value = get_register(&registers, value_register, pc, opcode)?.clone();
                        match self.heap.get_mut(object as usize) {
                            Some(HeapObject::Instance { fields, .. }) => {
                                fields.insert(field_key, value);
                            }
                            _ => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    "instance field target is not an object",
                                ))
                            }
                        }
                    }
                    pc += 2;
                }
                0x60..=0x6d => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let field_index = code_word(code, pc + 1, pc, opcode)? as u32;
                    let field_key = self
                        .dex
                        .field_key(field_index as usize)
                        .unwrap_or_else(|| format!("#{field_index}"));
                    if opcode <= 0x66 {
                        if let Some(field) = self.dex.field_id(field_index as usize) {
                            self.ensure_class_initialized(&field.class_name)?;
                        }
                        set_register(
                            &mut registers,
                            register,
                            self.static_fields
                                .get(&field_key)
                                .cloned()
                                .unwrap_or(Value::Null),
                            self,
                            pc,
                            opcode,
                        )?;
                    } else {
                        self.static_fields.insert(
                            field_key,
                            get_register(&registers, register, pc, opcode)?.clone(),
                        );
                    }
                    pc += 2;
                }
                0x6e | 0x6f | 0x71 | 0x72 => {
                    let method_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let args = invoke_args(
                        &registers,
                        instruction,
                        code_word(code, pc + 2, pc, opcode)?,
                        pc,
                        opcode,
                    )?;
                    let target = if opcode == 0x6e || opcode == 0x72 {
                        args.first()
                            .and_then(|value| match value {
                                Value::Object(id) => Some(*id),
                                _ => None,
                            })
                            .and_then(|object| self.instance_method_index(object, method_index))
                            .unwrap_or(method_index)
                    } else {
                        method_index
                    };
                    pending_result = self.call_method(target, args)?;
                    pc += 3;
                }
                0x70 => {
                    let method_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let args = invoke_args(
                        &registers,
                        instruction,
                        code_word(code, pc + 2, pc, opcode)?,
                        pc,
                        opcode,
                    )?;
                    pending_result = self.call_method(method_index, args)?;
                    pc += 3;
                }
                0x74..=0x78 => {
                    let method_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let count = ((instruction >> 8) & 0xff) as usize;
                    let first = code_word(code, pc + 2, pc, opcode)? as usize;
                    let args = (0..count)
                        .map(|offset| get_register(&registers, first + offset, pc, opcode))
                        .collect::<Result<Vec<_>, _>>()?;
                    pending_result = self.call_method(method_index, args)?;
                    pc += 3;
                }
                _ => return Err(self.error(pc, opcode, "opcode is not implemented by the VM")),
            }
        }
        Ok(Value::Void)
    }

    fn dispatch_framework(
        &mut self,
        class_name: &str,
        method_name: &str,
        args: &[Value],
    ) -> Result<Value, VmError> {
        if method_name == "<init>"
            && (class_name.starts_with("Lcom/badlogic/gdx/")
                || class_name.starts_with("Landroid/")
                || class_name.starts_with("Ljava/lang/ref/")
                || class_name == "Ljava/lang/Enum;"
                || class_name == "Ljava/util/ArrayList;"
                || class_name == "Ljava/util/LinkedList;"
                || class_name == "Ljava/util/HashMap;"
                || class_name == "Ljava/lang/StringBuilder;")
        {
            if class_name == "Ljava/util/ArrayList;"
                || class_name == "Ljava/util/LinkedList;"
                || class_name == "Ljava/util/HashMap;"
            {
                if let Some(Value::Object(receiver)) = args.first() {
                    if (*receiver as usize) < self.heap.len() {
                        self.heap[*receiver as usize] = HeapObject::Collection(Vec::new());
                    }
                }
            } else if class_name == "Ljava/lang/StringBuilder;" {
                if let Some(Value::Object(receiver)) = args.first() {
                    if (*receiver as usize) < self.heap.len() {
                        let initial = args
                            .get(1)
                            .and_then(|value| match value {
                                Value::String(text) => Some(text.clone()),
                                Value::Object(id) => match self.heap_object(*id) {
                                    Some(HeapObject::String(text)) => Some(text.clone()),
                                    _ => None,
                                },
                                _ => None,
                            })
                            .unwrap_or_default();
                        self.heap[*receiver as usize] = HeapObject::StringBuilder(initial);
                    }
                }
            }
            return Ok(Value::Void);
        }
        if class_name == "Ljava/lang/String;" {
            let value_of = |value: Option<&Value>| match value {
                Some(Value::String(value)) => value.clone(),
                Some(Value::Int(value)) => value.to_string(),
                Some(Value::Long(value)) => value.to_string(),
                Some(Value::Float(value)) => value.to_string(),
                Some(Value::Double(value)) => value.to_string(),
                Some(Value::Object(id)) => match self.heap_object(*id) {
                    Some(HeapObject::String(value)) => value.clone(),
                    Some(HeapObject::StringBuilder(value)) => value.clone(),
                    Some(HeapObject::Boxed(value)) => format!("{value:?}"),
                    Some(HeapObject::Class(value)) => value.clone(),
                    _ => "null".to_owned(),
                },
                _ => "null".to_owned(),
            };

            match method_name {
                "valueOf" => return Ok(Value::Object(self.alloc_string(value_of(args.first())))),
                "length" => return Ok(Value::Int(value_of(args.first()).chars().count() as i32)),
                "toString" => return Ok(Value::Object(object_arg(args, 0)?)),
                "concat" => {
                    let mut value = value_of(args.first());
                    value.push_str(&value_of(args.get(1)));
                    return Ok(Value::Object(self.alloc_string(value)));
                }
                "equals" => {
                    let left = value_of(args.first());
                    let right = value_of(args.get(1));
                    return Ok(Value::Int(i32::from(left == right)));
                }
                "startsWith" | "endsWith" | "contains" => {
                    let left = value_of(args.first());
                    let right = value_of(args.get(1));
                    let matched = match method_name {
                        "startsWith" => left.starts_with(&right),
                        "endsWith" => left.ends_with(&right),
                        _ => left.contains(&right),
                    };
                    return Ok(Value::Int(i32::from(matched)));
                }

                _ => {}
            }
        }
        if class_name == "Ljava/lang/StringBuilder;" {
            let receiver = object_arg(args, 0)?;
            match method_name {
                "append" => {
                    let text = match args.get(1) {
                        Some(Value::String(value)) => value.clone(),
                        Some(Value::Int(value)) => value.to_string(),
                        Some(Value::Long(value)) => value.to_string(),
                        Some(Value::Float(value)) => value.to_string(),
                        Some(Value::Double(value)) => value.to_string(),
                        Some(Value::Object(id)) => match self.heap_object(*id) {
                            Some(HeapObject::String(value)) => value.clone(),
                            Some(HeapObject::StringBuilder(value)) => value.clone(),
                            _ => String::new(),
                        },
                        _ => String::new(),
                    };
                    if let Some(HeapObject::StringBuilder(value)) =
                        self.heap.get_mut(receiver as usize)
                    {
                        value.push_str(&text);
                    }
                    return Ok(Value::Object(receiver));
                }
                "toString" => {
                    let value = match self.heap_object(receiver) {
                        Some(HeapObject::StringBuilder(value)) => value.clone(),
                        Some(HeapObject::String(value)) => value.clone(),
                        _ => String::new(),
                    };
                    return Ok(Value::Object(self.alloc_string(value)));
                }
                "length" => {
                    return Ok(Value::Int(match self.heap_object(receiver) {
                        Some(HeapObject::StringBuilder(value)) => value.chars().count() as i32,
                        _ => 0,
                    }))
                }
                _ => {}
            }
        }
        if class_name == "Ljava/util/ArrayList;"
            || class_name == "Ljava/util/LinkedList;"
            || class_name == "Ljava/util/HashMap;"
            || class_name == "Ljava/util/Collection;"
            || class_name == "Ljava/util/List;"
            || class_name == "Ljava/util/AbstractList;"
            || class_name == "Ljava/util/AbstractCollection;"
        {
            let receiver = object_arg(args, 0)?;
            match method_name {
                "size" => {
                    return Ok(Value::Int(match self.heap_object(receiver) {
                        Some(HeapObject::Collection(values)) => values.len() as i32,
                        _ => 0,
                    }))
                }
                "isEmpty" => {
                    return Ok(Value::Int(match self.heap_object(receiver) {
                        Some(HeapObject::Collection(values)) => i32::from(values.is_empty()),
                        _ => 1,
                    }))
                }
                "add" => {
                    if let Some(HeapObject::Collection(values)) =
                        self.heap.get_mut(receiver as usize)
                    {
                        values.push(args.get(1).cloned().unwrap_or(Value::Null));
                    }
                    return Ok(Value::Int(1));
                }
                "get" => {
                    let index = int_arg(args, 1)? as usize;
                    return Ok(match self.heap_object(receiver) {
                        Some(HeapObject::Collection(values)) => {
                            values.get(index).cloned().unwrap_or(Value::Null)
                        }
                        _ => Value::Null,
                    });
                }
                _ => return Ok(Value::Void),
            }
        }
        if class_name == "Ljava/lang/Class;" && method_name == "forName" {
            let requested = self.string_arg(args, 0)?;
            let descriptor = if requested.starts_with('L') && requested.ends_with(';') {
                requested
            } else {
                format!("L{};", requested.replace('.', "/"))
            };
            return Ok(Value::Object(self.alloc(HeapObject::Class(descriptor))));
        }
        let result = match (class_name, method_name) {
            _ => {
                return Err(self.error(
                    0,
                    0,
                    format!("framework method {class_name}->{method_name} is not implemented"),
                ))
            }
        };
        Ok(match result {
            FrameworkResult::Void => Value::Void,
            FrameworkResult::Int(value) => Value::Int(value),
            FrameworkResult::Long(value) => Value::Long(value),
            FrameworkResult::Bool(value) => Value::Int(i32::from(value)),
            FrameworkResult::Object(value) => {
                if value == 0 {
                    Value::Null
                } else {
                    Value::Object(value)
                }
            }
            FrameworkResult::String(value) => Value::String(value),
        })
    }

    fn dispatch_gdx(
        &mut self,
        class_name: &str,
        method_name: &str,
        args: &[Value],
    ) -> Result<Value, VmError> {
        let result = match (class_name, method_name) {
            (_, "<clinit>") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "getType")
            | ("Lcom/badlogic/gdx/Application;", "getType") => FrameworkResult::Object(0),
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "initializeForView") => {
                let view = self.framework.alloc_view("Landroid/opengl/GLSurfaceView;");
                self.framework.gdx_view = Some(view);
                self.framework.gdx_listener = args.first().and_then(|value| match value {
                    Value::Object(id) => Some(*id),
                    _ => None,
                });
                self.framework.surface_size = (320, 480);
                self.framework
                    .surface_events
                    .push(format!("created:{view}"));
                self.framework
                    .surface_events
                    .push(format!("changed:{view}:0:320x480"));
                FrameworkResult::Object(view)
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidGraphics;", "getView") => {
                let view = self.framework.gdx_view.unwrap_or_else(|| {
                    let view = self.framework.alloc_view("Landroid/opengl/GLSurfaceView;");
                    self.framework.gdx_view = Some(view);
                    view
                });
                FrameworkResult::Object(view)
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "getWindow") => {
                FrameworkResult::Object(self.alloc_instance("Landroid/view/Window;"))
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "requestWindowFeature") => {
                object_arg(args, 0)?;
                FrameworkResult::Bool(true)
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "setContentView") => {
                let activity = object_arg(args, 0)?;
                let view = object_arg(args, 1)?;
                self.framework_call(FrameworkCall::SetContentView { activity, view })?
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "getInput") => {
                let value = self.framework.gdx_input.unwrap_or_else(|| {
                    let value = self.alloc_instance("Lcom/badlogic/gdx/Input;");
                    self.framework.gdx_input = Some(value);
                    value
                });
                FrameworkResult::Object(value)
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "getAudio") => {
                let value = self.framework.gdx_audio.unwrap_or_else(|| {
                    let value = self.alloc_instance("Lcom/badlogic/gdx/Audio;");
                    self.framework.gdx_audio = Some(value);
                    value
                });
                FrameworkResult::Object(value)
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "getFiles") => {
                let value = self.framework.gdx_files.unwrap_or_else(|| {
                    let value = self.alloc_instance("Lcom/badlogic/gdx/Files;");
                    self.framework.gdx_files = Some(value);
                    value
                });
                FrameworkResult::Object(value)
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "getGraphics") => {
                let value = self.framework.gdx_graphics.unwrap_or_else(|| {
                    let value = self.alloc_instance("Lcom/badlogic/gdx/Graphics;");
                    self.framework.gdx_graphics = Some(value);
                    value
                });
                FrameworkResult::Object(value)
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "getAssets") => {
                FrameworkResult::Object(self.alloc_instance("Landroid/content/res/AssetManager;"))
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "getPreferences") => {
                FrameworkResult::Object(self.alloc_instance("Lcom/badlogic/gdx/Preferences;"))
            }
            ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "createWakeLock")
            | ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "onCreate")
            | ("Lcom/badlogic/gdx/backends/android/AndroidApplication;", "postRunnable")
            | ("Lcom/badlogic/gdx/ApplicationListener;", "create")
            | ("Lcom/badlogic/gdx/ApplicationListener;", "resize")
            | ("Lcom/badlogic/gdx/ApplicationListener;", "render")
            | ("Lcom/badlogic/gdx/ApplicationListener;", "pause")
            | ("Lcom/badlogic/gdx/ApplicationListener;", "resume")
            | ("Lcom/badlogic/gdx/ApplicationListener;", "dispose") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/Graphics;", "getWidth") => {
                FrameworkResult::Int(self.framework.surface_size.0)
            }
            ("Lcom/badlogic/gdx/Graphics;", "getHeight") => {
                FrameworkResult::Int(self.framework.surface_size.1)
            }
            ("Lcom/badlogic/gdx/Graphics;", "getDeltaTime") => FrameworkResult::Int(0),
            ("Lcom/badlogic/gdx/Graphics;", "getFramesPerSecond") => FrameworkResult::Int(60),
            ("Lcom/badlogic/gdx/Graphics;", "getGL10")
            | ("Lcom/badlogic/gdx/Graphics;", "getGLCommon") => {
                FrameworkResult::Object(self.alloc_instance("Lcom/badlogic/gdx/graphics/GL10;"))
            }
            ("Lcom/badlogic/gdx/Input;", "isTouched") => FrameworkResult::Bool(false),
            ("Lcom/badlogic/gdx/Input;", "isKeyPressed") => FrameworkResult::Bool(false),
            ("Lcom/badlogic/gdx/Input;", "setInputProcessor") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/Files;", "internal")
            | ("Lcom/badlogic/gdx/Files;", "external")
            | ("Lcom/badlogic/gdx/Files;", "local") => {
                FrameworkResult::Object(self.alloc_instance("Lcom/badlogic/gdx/files/FileHandle;"))
            }
            ("Lcom/badlogic/gdx/files/FileHandle;", "exists") => FrameworkResult::Bool(false),
            ("Lcom/badlogic/gdx/files/FileHandle;", "length") => FrameworkResult::Long(0),
            ("Lcom/badlogic/gdx/files/FileHandle;", "readBytes") => FrameworkResult::Object(0),
            ("Lcom/badlogic/gdx/files/FileHandle;", "readString") => {
                FrameworkResult::String(String::new())
            }
            ("Lcom/badlogic/gdx/audio/Sound;", "play")
            | ("Lcom/badlogic/gdx/audio/Sound;", "loop")
            | ("Lcom/badlogic/gdx/audio/Sound;", "stop") => FrameworkResult::Long(0),
            ("Lcom/badlogic/gdx/audio/Music;", "play")
            | ("Lcom/badlogic/gdx/audio/Music;", "stop")
            | ("Lcom/badlogic/gdx/audio/Music;", "setLooping")
            | ("Lcom/badlogic/gdx/audio/Music;", "dispose")
            | ("Lcom/badlogic/gdx/audio/Music;", "setVolume") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/audio/Music;", "isPlaying") => FrameworkResult::Bool(false),
            ("Lcom/badlogic/gdx/audio/Sound;", "dispose") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "findRegion") => {
                FrameworkResult::Object(
                    self.alloc_instance("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;"),
                )
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "findRegions") => {
                FrameworkResult::Object(self.alloc_collection())
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "createSprite")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "newSprite") => {
                FrameworkResult::Object(
                    self.alloc_instance("Lcom/badlogic/gdx/graphics/g2d/Sprite;"),
                )
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "dispose") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/graphics/g2d/TextureRegion;", "getRegionWidth")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;", "getRegionWidth")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureRegion;", "getRegionHeight")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;", "getRegionHeight")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureRegion;", "getRegionX")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;", "getRegionX")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureRegion;", "getRegionY")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;", "getRegionY") => {
                FrameworkResult::Int(0)
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureRegion;", "getTexture")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;", "getTexture") => {
                FrameworkResult::Object(self.alloc_instance("Lcom/badlogic/gdx/graphics/Texture;"))
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureRegion;", "setRegion")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;", "setRegion") => {
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setOrigin")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setPosition")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setRotation")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setScale")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setSize")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setBounds")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setColor")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setRegion")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setTexture")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "translate")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "rotate") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getX")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getY")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getWidth")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getHeight")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getRotation")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getScaleX")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getScaleY") => FrameworkResult::Int(0),
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getColor") => {
                FrameworkResult::Object(self.alloc_instance("Lcom/badlogic/gdx/graphics/Color;"))
            }
            ("Lcom/badlogic/gdx/graphics/Texture;", "dispose")
            | ("Lcom/badlogic/gdx/graphics/Texture;", "setFilter")
            | ("Lcom/badlogic/gdx/graphics/Texture;", "setWrap") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/graphics/Texture;", "getWidth")
            | ("Lcom/badlogic/gdx/graphics/Texture;", "getHeight") => FrameworkResult::Int(256),
            ("Lcom/badlogic/gdx/Application;", "getPreferences") => {
                FrameworkResult::Object(self.alloc_instance("Lcom/badlogic/gdx/Preferences;"))
            }
            ("Lcom/badlogic/gdx/Preferences;", "getString") => {
                FrameworkResult::String(String::new())
            }
            ("Lcom/badlogic/gdx/Preferences;", "getBoolean") => FrameworkResult::Bool(false),
            ("Lcom/badlogic/gdx/Preferences;", "getInteger") => FrameworkResult::Int(0),
            ("Lcom/badlogic/gdx/Preferences;", "putString")
            | ("Lcom/badlogic/gdx/Preferences;", "putBoolean")
            | ("Lcom/badlogic/gdx/Preferences;", "putInteger")
            | ("Lcom/badlogic/gdx/Preferences;", "flush") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/Application;", "getAudio") => {
                FrameworkResult::Object(self.alloc_instance("Lcom/badlogic/gdx/Audio;"))
            }
            ("Lcom/badlogic/gdx/Application;", "getGraphics") => {
                let value = self.framework.gdx_graphics.unwrap_or_else(|| {
                    let value = self.alloc_instance("Lcom/badlogic/gdx/Graphics;");
                    self.framework.gdx_graphics = Some(value);
                    value
                });
                FrameworkResult::Object(value)
            }
            ("Lcom/badlogic/gdx/Application;", "log") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glClearColor")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glClearColor") => {
                let color = Rgba8 {
                    r: (float_arg(args, 1)? * 255.0).clamp(0.0, 255.0) as u8,
                    g: (float_arg(args, 2)? * 255.0).clamp(0.0, 255.0) as u8,
                    b: (float_arg(args, 3)? * 255.0).clamp(0.0, 255.0) as u8,
                    a: (float_arg(args, 4)? * 255.0).clamp(0.0, 255.0) as u8,
                };
                self.framework.gles.clear_color(color);
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glClear")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glClear") => {
                self.framework.gles.clear_mask(int_arg(args, 1)? as u32);
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glEnable")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glEnable") => {
                self.framework.gles.enable(int_arg(args, 1)? as u32);
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glDisable")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glDisable") => {
                self.framework.gles.disable(int_arg(args, 1)? as u32);
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glBlendFunc")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glBlendFunc") => {
                self.framework
                    .gles
                    .blend_func(int_arg(args, 1)? as u32, int_arg(args, 2)? as u32);
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glBindTexture")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glBindTexture") => {
                self.framework
                    .gles
                    .bind_texture(int_arg(args, 1)? as u32, int_arg(args, 2)? as u32);
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glViewport")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glViewport") => {
                self.framework.gles.viewport(
                    int_arg(args, 1)?,
                    int_arg(args, 2)?,
                    int_arg(args, 3)?.max(0) as u32,
                    int_arg(args, 4)?.max(0) as u32,
                );
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glDrawArrays")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glDrawArrays") => {
                self.framework.gles.draw_arrays(
                    int_arg(args, 1)? as u32,
                    int_arg(args, 2)?,
                    int_arg(args, 3)?,
                );
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glDrawElements")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glDrawElements") => {
                self.framework.gles.draw_elements(
                    int_arg(args, 1)? as u32,
                    int_arg(args, 2)?,
                    int_arg(args, 3)? as u32,
                );
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glGetString")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glGetString") => {
                FrameworkResult::String("DonutHLE GLES 1.0 software renderer".to_owned())
            }
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glLoadIdentity")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glLoadIdentity")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glLoadMatrixf")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glLoadMatrixf")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glMatrixMode")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glMatrixMode")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glTexImage2D")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glTexImage2D")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glTexParameterf")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glTexParameterf")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glTexSubImage2D")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glTexSubImage2D")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glCompressedTexImage2D")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glCompressedTexImage2D")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glPixelStorei")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glPixelStorei")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glDepthMask")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glDepthMask")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glScissor")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glScissor")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glActiveTexture")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glActiveTexture")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glClientActiveTexture")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glEnableClientState")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glDisableClientState")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glVertexPointer")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glColorPointer")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glTexCoordPointer")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glNormalPointer")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glMaterialfv")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glPointSize") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/graphics/GLCommon;", "glGenTextures")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glGenTextures")
            | ("Lcom/badlogic/gdx/graphics/GLCommon;", "glDeleteTextures")
            | ("Lcom/badlogic/gdx/graphics/GL10;", "glDeleteTextures") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/math/MathUtils;", "random") => {
                let value = match args.len() {
                    1 => float_arg(args, 0)?.mul_add(0.5, 0.5),
                    2 => {
                        let low = int_arg(args, 0)?;
                        let high = int_arg(args, 1)?;
                        if high < low {
                            return Err(self.error(
                                0,
                                0,
                                "MathUtils.random upper bound is below lower bound",
                            ));
                        }
                        (low + (self.executed_steps as i32 % (high - low + 1))) as f32
                    }
                    _ => {
                        ((self.executed_steps as i32)
                            .wrapping_mul(1103515245)
                            .wrapping_add(12345)
                            & 0x7fff) as f32
                    }
                };
                FrameworkResult::Int(value as i32)
            }
            ("Lcom/badlogic/gdx/math/MathUtils;", "randomBoolean") => {
                FrameworkResult::Bool(self.executed_steps.is_multiple_of(2))
            }
            ("Lcom/badlogic/gdx/math/MathUtils;", "round") => {
                FrameworkResult::Int(float_arg(args, 0)?.round() as i32)
            }
            ("Lcom/badlogic/gdx/math/MathUtils;", "sin") => {
                FrameworkResult::Int(float_arg(args, 0)?.sin().to_bits() as i32)
            }
            ("Lcom/badlogic/gdx/graphics/Color;", "toFloatBits") => {
                FrameworkResult::Int(int_arg(args, 0)?)
            }
            ("Lcom/badlogic/gdx/math/Vector3;", "set") => {
                let receiver = match args.first() {
                    Some(Value::Object(id)) => *id,
                    _ => return Ok(Value::Void),
                };
                let _x = float_arg(args, 1)?;
                let _y = float_arg(args, 2)?;
                let _z = float_arg(args, 3)?;
                FrameworkResult::Object(receiver)
            }
            ("Lcom/badlogic/gdx/Graphics;", "setVSync")
            | ("Lcom/badlogic/gdx/Graphics;", "setContinuousRendering")
            | ("Lcom/badlogic/gdx/Graphics;", "setDisplayMode")
            | ("Lcom/badlogic/gdx/Graphics;", "setTitle")
            | ("Lcom/badlogic/gdx/Graphics;", "setResizable")
            | ("Lcom/badlogic/gdx/Graphics;", "setFullscreenMode")
            | ("Lcom/badlogic/gdx/Graphics;", "setWindowedMode")
            | ("Lcom/badlogic/gdx/Graphics;", "setSystemCursor")
            | ("Lcom/badlogic/gdx/Graphics;", "setUndecorated")
            | ("Lcom/badlogic/gdx/Graphics;", "setBorderlessWindow")
            | ("Lcom/badlogic/gdx/Graphics;", "setForegroundFPS")
            | ("Lcom/badlogic/gdx/Graphics;", "setIdleFPS") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/Graphics;", "isFullscreen")
            | ("Lcom/badlogic/gdx/Graphics;", "isGL30Available")
            | ("Lcom/badlogic/gdx/Graphics;", "isGL31Available")
            | ("Lcom/badlogic/gdx/Graphics;", "isGL32Available") => FrameworkResult::Bool(false),
            ("Lcom/badlogic/gdx/Graphics;", "getDensity")
            | ("Lcom/badlogic/gdx/Graphics;", "getPpcX")
            | ("Lcom/badlogic/gdx/Graphics;", "getPpcY") => FrameworkResult::Int(1),
            ("Lcom/badlogic/gdx/graphics/OrthographicCamera;", "update")
            | ("Lcom/badlogic/gdx/graphics/OrthographicCamera;", "apply")
            | ("Lcom/badlogic/gdx/graphics/OrthographicCamera;", "translate")
            | ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "begin")
            | ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "end")
            | ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "dispose")
            | ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "enableBlending")
            | ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "disableBlending")
            | ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "setProjectionMatrix")
            | ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "setColor")
            | ("Lcom/badlogic/gdx/graphics/g2d/BitmapFont;", "draw") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/Input;", "setCatchBackKey")
            | ("Lcom/badlogic/gdx/Input;", "setOnscreenKeyboardVisible")
            | ("Lcom/badlogic/gdx/Input;", "getTextInput") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/Input;", "isPeripheralAvailable") => FrameworkResult::Bool(false),
            ("Lcom/badlogic/gdx/Input;", "getCurrentEventTime") => FrameworkResult::Long(0),
            ("Lcom/badlogic/gdx/Input;", "getRotation")
            | ("Lcom/badlogic/gdx/Input;", "getFreePointerIndex")
            | ("Lcom/badlogic/gdx/Input;", "lookUpPointerIndex") => FrameworkResult::Int(0),
            ("Lcom/badlogic/gdx/Input;", "getAccelerometerX")
            | ("Lcom/badlogic/gdx/Input;", "getAccelerometerY")
            | ("Lcom/badlogic/gdx/Input;", "getAccelerometerZ")
            | ("Lcom/badlogic/gdx/Input;", "getAzimuth")
            | ("Lcom/badlogic/gdx/Input;", "getPitch")
            | ("Lcom/badlogic/gdx/Input;", "getRoll") => FrameworkResult::Int(0),
            _ => {
                return Err(self.error(
                    0,
                    0,
                    format!("GDX method {class_name}->{method_name} is not implemented"),
                ))
            }
        };
        Ok(match result {
            FrameworkResult::Void => Value::Void,
            FrameworkResult::Int(value) => Value::Int(value),
            FrameworkResult::Long(value) => Value::Long(value),
            FrameworkResult::Bool(value) => Value::Int(i32::from(value)),
            FrameworkResult::Object(value) => {
                if value == 0 {
                    Value::Null
                } else {
                    Value::Object(value)
                }
            }
            FrameworkResult::String(value) => Value::String(value),
        })
    }

    fn framework_call(&mut self, call: FrameworkCall) -> Result<FrameworkResult, VmError> {
        self.framework
            .dispatch(call)
            .map_err(|message| self.error(0, 0, message))
    }

    fn string_arg(&self, args: &[Value], index: usize) -> Result<String, VmError> {
        match args.get(index) {
            Some(Value::String(value)) => Ok(value.clone()),
            Some(Value::Object(id)) => match self.heap_object(*id) {
                Some(HeapObject::String(value)) => Ok(value.clone()),
                _ => Err(self.error(
                    0,
                    0,
                    format!("framework argument {index} is object {id}, expected java.lang.String"),
                )),
            },
            _ => Err(self.error(0, 0, format!("framework argument {index} is not a string"))),
        }
    }

    fn alloc(&mut self, object: HeapObject) -> ObjectId {
        let id = self.heap.len() as ObjectId;
        self.heap.push(object);
        id
    }

    fn error(&self, pc: usize, opcode: u8, message: impl Into<String>) -> VmError {
        VmError {
            pc,
            opcode,
            message: message.into(),
        }
    }
}

fn code_word(code: &CodeItem, index: usize, pc: usize, opcode: u8) -> Result<u16, VmError> {
    code.instructions
        .get(index)
        .copied()
        .ok_or_else(|| VmError {
            pc,
            opcode,
            message: "instruction payload is truncated".to_owned(),
        })
}

fn get_register(
    registers: &[Value],
    index: usize,
    pc: usize,
    opcode: u8,
) -> Result<Value, VmError> {
    registers.get(index).cloned().ok_or_else(|| VmError {
        pc,
        opcode,
        message: format!("register v{index} is outside the frame"),
    })
}

fn set_register(
    registers: &mut [Value],
    index: usize,
    value: Value,
    _vm: &Vm<'_>,
    pc: usize,
    opcode: u8,
) -> Result<(), VmError> {
    *registers.get_mut(index).ok_or_else(|| VmError {
        pc,
        opcode,
        message: format!("register v{index} is outside the frame"),
    })? = value;
    Ok(())
}

fn two_registers(instruction: u16) -> (usize, usize) {
    (
        ((instruction >> 8) & 0x0f) as usize,
        ((instruction >> 12) & 0x0f) as usize,
    )
}

fn three_registers(instruction: u16, word: u16) -> (usize, usize, usize) {
    (
        ((instruction >> 8) & 0xff) as usize,
        (word & 0xff) as usize,
        (word >> 8) as usize,
    )
}

fn invoke_args(
    registers: &[Value],
    instruction: u16,
    word: u16,
    pc: usize,
    opcode: u8,
) -> Result<Vec<Value>, VmError> {
    let count = ((instruction >> 12) & 0x0f) as usize;
    let candidates = [
        (word & 0x0f) as usize,
        ((word >> 4) & 0x0f) as usize,
        ((word >> 8) & 0x0f) as usize,
        ((word >> 12) & 0x0f) as usize,
        ((instruction >> 8) & 0x0f) as usize,
    ];
    candidates[..count.min(candidates.len())]
        .iter()
        .map(|index| get_register(registers, *index, pc, opcode))
        .collect()
}

fn get_object(
    registers: &[Value],
    register: usize,
    vm: &Vm<'_>,
    pc: usize,
    opcode: u8,
) -> Result<ObjectId, VmError> {
    match get_register(registers, register, pc, opcode)? {
        Value::Object(id) => Ok(id),
        Value::Null => Err(vm.error(pc, opcode, "null object reference")),
        value => Err(vm.error(
            pc,
            opcode,
            format!("value in v{register} is not an object: {value:?}"),
        )),
    }
}

fn as_int(value: Value, pc: usize, opcode: u8) -> Result<i32, VmError> {
    match value {
        Value::Int(value) => Ok(value),
        Value::Null => Ok(0),
        _ => Err(VmError {
            pc,
            opcode,
            message: "value is not an integer".to_owned(),
        }),
    }
}

fn as_long(value: Value, pc: usize, opcode: u8) -> Result<i64, VmError> {
    match value {
        Value::Long(value) => Ok(value),
        Value::Int(value) => Ok(value as i64),
        Value::Float(value) => Ok(value as i64),
        Value::Double(value) => Ok(value as i64),
        Value::Null => Ok(0),
        _ => Err(VmError {
            pc,
            opcode,
            message: "value is not a long".to_owned(),
        }),
    }
}

fn as_float(value: Value, pc: usize, opcode: u8) -> Result<f32, VmError> {
    match value {
        Value::Float(value) => Ok(value),
        Value::Double(value) => Ok(value as f32),
        Value::Int(value) => Ok(value as f32),
        Value::Long(value) => Ok(value as f32),
        Value::Null => Ok(0.0),
        _ => Err(VmError {
            pc,
            opcode,
            message: "value is not a float".to_owned(),
        }),
    }
}

fn as_double(value: Value, pc: usize, opcode: u8) -> Result<f64, VmError> {
    match value {
        Value::Double(value) => Ok(value),
        Value::Float(value) => Ok(value as f64),
        Value::Int(value) => Ok(value as f64),
        Value::Long(value) => Ok(value as f64),
        Value::Null => Ok(0.0),
        _ => Err(VmError {
            pc,
            opcode,
            message: "value is not a double".to_owned(),
        }),
    }
}

fn float_arg(args: &[Value], index: usize) -> Result<f32, VmError> {
    match args.get(index) {
        Some(Value::Float(value)) => Ok(*value),
        Some(Value::Double(value)) => Ok(*value as f32),
        Some(Value::Int(value)) => Ok(*value as f32),
        Some(Value::Long(value)) => Ok(*value as f32),
        _ => Err(VmError {
            pc: 0,
            opcode: 0,
            message: format!("framework argument {index} is not numeric"),
        }),
    }
}

fn int_arg(args: &[Value], index: usize) -> Result<i32, VmError> {
    match args.get(index) {
        Some(Value::Int(value)) => Ok(*value),
        Some(Value::Long(value)) => Ok(*value as i32),
        Some(Value::Float(value)) => Ok(*value as i32),
        Some(Value::Double(value)) => Ok(*value as i32),
        _ => Err(VmError {
            pc: 0,
            opcode: 0,
            message: format!("framework argument {index} is not an integer"),
        }),
    }
}

fn object_arg(args: &[Value], index: usize) -> Result<ObjectId, VmError> {
    match args.get(index) {
        Some(Value::Object(value)) => Ok(*value),
        _ => Err(VmError {
            pc: 0,
            opcode: 0,
            message: format!("framework argument {index} is not an object"),
        }),
    }
}

fn string_arg(args: &[Value], index: usize) -> Result<String, VmError> {
    match args.get(index) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Object(id)) => Err(VmError {
            pc: 0,
            opcode: 0,
            message: format!(
                "framework argument {index} is object {id}, expected java.lang.String"
            ),
        }),
        _ => Err(VmError {
            pc: 0,
            opcode: 0,
            message: format!("framework argument {index} is not a string"),
        }),
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    left == right
}

fn branch_target(
    pc: usize,
    offset: i32,
    len: usize,
    at: usize,
    opcode: u8,
) -> Result<usize, VmError> {
    let target = pc as i64 + offset as i64;
    if target < 0 || target as usize >= len {
        if offset > 0 {
            return Ok(pc.saturating_add(1).min(len.saturating_sub(1)));
        }
        return Err(VmError {
            pc: at,
            opcode,
            message: format!(
                "branch target outside code: pc={pc} offset={offset} target={target} len={len}"
            ),
        });
    }
    Ok(target as usize)
}
