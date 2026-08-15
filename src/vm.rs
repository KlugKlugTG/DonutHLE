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
    Instance {
        class_name: String,
        fields: HashMap<u32, Value>,
    },
    Array {
        component: String,
        values: Vec<Value>,
    },
    String(String),
    Class(String),
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
    static_fields: HashMap<u32, Value>,
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

    pub fn run_method(&mut self, method_index: usize, args: Vec<Value>) -> Result<Value, VmError> {
        self.call_method(method_index, args)
    }

    pub fn run_named_method(
        &mut self,
        class_name: &str,
        method_name: &str,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let method_index = self
            .dex
            .methods
            .iter()
            .position(|method| method.class_name == class_name && method.name == method_name)
            .ok_or_else(|| {
                self.error(
                    0,
                    0,
                    format!("method {class_name}->{method_name} not found"),
                )
            })?;
        self.call_method(method_index, args)
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
        if method.class_name.starts_with("Landroid/") || method.class_name.starts_with("Ljava/") {
            return self.dispatch_framework(&method.class_name, &method.name, &args);
        }
        let code = self
            .method_code_by_index(method_index)
            .ok_or_else(|| {
                self.error(
                    0,
                    0,
                    format!(
                        "method {} has no code (abstract/native methods are not executable)",
                        method.name
                    ),
                )
            })?
            .clone();
        self.call_depth += 1;
        let result = self.execute_code(&code, args);
        self.call_depth -= 1;
        result
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
                0x01..=0x09 => {
                    let (dest, source) = two_registers(instruction);
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 1;
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
                    set_register(
                        &mut registers,
                        dest,
                        pending_result.clone(),
                        self,
                        pc,
                        opcode,
                    )?;
                    set_register(&mut registers, dest + 1, Value::Int(0), self, pc, opcode)?;
                    pc += 1;
                }
                0x0e => return Ok(Value::Void),
                0x0f..=0x11 => {
                    return get_register(
                        &registers,
                        ((instruction >> 8) & 0xff) as usize,
                        pc,
                        opcode,
                    );
                }
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
                0x1a | 0x1b => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let index = if opcode == 0x1a {
                        code_word(code, pc + 1, pc, opcode)? as usize
                    } else {
                        code_word(code, pc + 1, pc, opcode)? as usize
                            | (code_word(code, pc + 2, pc, opcode)? as usize) << 16
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
                0x21 => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
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
                    let object = self.alloc_instance(class_name.clone());
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
                0x27 => {
                    return Err(self.error(
                        pc,
                        opcode,
                        "throw is not supported by the API-4 runtime",
                    ))
                }
                0x28 => {
                    pc = branch_target(
                        pc,
                        ((instruction >> 8) as i8) as i32,
                        code.instructions.len(),
                        pc,
                        opcode,
                    )?
                }
                0x29 => {
                    pc = branch_target(
                        pc,
                        code_word(code, pc + 1, pc, opcode)? as i16 as i32,
                        code.instructions.len(),
                        pc,
                        opcode,
                    )?
                }
                0x2a => {
                    let low = code_word(code, pc + 1, pc, opcode)? as i32;
                    let high = code_word(code, pc + 2, pc, opcode)? as i32;
                    pc = branch_target(pc, low | high << 16, code.instructions.len(), pc, opcode)?;
                }
                0x32..=0x37 => {
                    let (left, right) = two_registers(instruction);
                    let left = get_register(&registers, left, pc, opcode)?;
                    let right = get_register(&registers, right, pc, opcode)?;
                    let equal = values_equal(&left, &right);
                    let take = if opcode.is_multiple_of(2) {
                        equal
                    } else {
                        !equal
                    };
                    pc = if take {
                        branch_target(
                            pc,
                            code_word(code, pc + 1, pc, opcode)? as i16 as i32,
                            code.instructions.len(),
                            pc,
                            opcode,
                        )?
                    } else {
                        pc + 2
                    };
                }
                0x38..=0x3d => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let value = get_register(&registers, register, pc, opcode)?;
                    let zero = matches!(value, Value::Null) || matches!(value, Value::Int(0));
                    let take = if opcode.is_multiple_of(2) {
                        zero
                    } else {
                        !zero
                    };
                    pc = if take {
                        branch_target(
                            pc,
                            code_word(code, pc + 1, pc, opcode)? as i16 as i32,
                            code.instructions.len(),
                            pc,
                            opcode,
                        )?
                    } else {
                        pc + 2
                    };
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
                    let heap_index = array as usize;
                    let Some(object) = self.heap.get_mut(heap_index) else {
                        return Err(self.error(pc, opcode, "array target is not an array"));
                    };
                    match object {
                        HeapObject::Array { values, .. } => {
                            let Some(slot) = values.get_mut(index) else {
                                return Err(self.error(pc, opcode, "array index out of bounds"));
                            };
                            *slot = value;
                        }
                        _ => return Err(self.error(pc, opcode, "array target is not an array")),
                    }
                    pc += 2;
                }
                0x52..=0x5f => {
                    let (first, second) = two_registers(instruction);
                    let field_index = code_word(code, pc + 1, pc, opcode)? as u32;
                    if opcode <= 0x58 {
                        let object = get_object(&registers, second, self, pc, opcode)?;
                        let value = match self.heap_object(object) {
                            Some(HeapObject::Instance { fields, .. }) => {
                                fields.get(&field_index).cloned().unwrap_or(Value::Null)
                            }
                            _ => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    "instance field target is not an object",
                                ))
                            }
                        };
                        set_register(&mut registers, first, value, self, pc, opcode)?;
                    } else {
                        let object = get_object(&registers, first, self, pc, opcode)?;
                        let value = get_register(&registers, second, pc, opcode)?.clone();
                        match self.heap.get_mut(object as usize) {
                            Some(HeapObject::Instance { fields, .. }) => {
                                fields.insert(field_index, value);
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
                0x60..=0x66 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let field_index = code_word(code, pc + 1, pc, opcode)? as u32;
                    if opcode <= 0x62 {
                        set_register(
                            &mut registers,
                            register,
                            self.static_fields
                                .get(&field_index)
                                .cloned()
                                .unwrap_or(Value::Null),
                            self,
                            pc,
                            opcode,
                        )?;
                    } else {
                        self.static_fields.insert(
                            field_index,
                            get_register(&registers, register, pc, opcode)?.clone(),
                        );
                    }
                    pc += 2;
                }
                0x6e..=0x72 => {
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
                0x7b | 0x7c => {
                    let (dest, source) = two_registers(instruction);
                    let value = as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    set_register(
                        &mut registers,
                        dest,
                        Value::Int(if opcode == 0x7b { -value } else { !value }),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 1;
                }
                0x90..=0x9a => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    let word = code_word(code, pc + 1, pc, opcode)?;
                    let left = (word & 0xff) as usize;
                    let right = (word >> 8) as usize;
                    let a = as_int(get_register(&registers, left, pc, opcode)?, pc, opcode)?;
                    let b = as_int(get_register(&registers, right, pc, opcode)?, pc, opcode)?;
                    let value = match opcode {
                        0x90 => a.wrapping_add(b),
                        0x91 => a.wrapping_sub(b),
                        0x92 => a.wrapping_mul(b),
                        0x93 => a
                            .checked_div(b)
                            .ok_or_else(|| self.error(pc, opcode, "division by zero"))?,
                        0x94 => a
                            .checked_rem(b)
                            .ok_or_else(|| self.error(pc, opcode, "remainder by zero"))?,
                        0x95 => a & b,
                        0x96 => a | b,
                        0x97 => a ^ b,
                        0x98 => a.wrapping_shl((b & 31) as u32),
                        0x99 => a.wrapping_shr((b & 31) as u32),
                        _ => ((a as u32).wrapping_shr((b & 31) as u32)) as i32,
                    };
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 2;
                }
                0xd8..=0xe2 => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    let source = ((instruction >> 12) & 0x0f) as usize;
                    let literal = (code_word(code, pc + 1, pc, opcode)? as i8) as i32;
                    let a = as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    let value = match opcode {
                        0xd8 => a.wrapping_add(literal),
                        0xd9 => literal.wrapping_sub(a),
                        0xda => a.wrapping_mul(literal),
                        0xdb => a
                            .checked_div(literal)
                            .ok_or_else(|| self.error(pc, opcode, "division by zero"))?,
                        0xdc => a
                            .checked_rem(literal)
                            .ok_or_else(|| self.error(pc, opcode, "remainder by zero"))?,
                        0xdd => a & literal,
                        0xde => a | literal,
                        0xdf => a ^ literal,
                        0xe0 => a.wrapping_shl((literal & 31) as u32),
                        0xe1 => a.wrapping_shr((literal & 31) as u32),
                        _ => ((a as u32).wrapping_shr((literal & 31) as u32)) as i32,
                    };
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 2;
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
        if method_name == "<init>" {
            return Ok(Value::Void);
        }
        if class_name == "Landroid/app/Activity;"
            && matches!(
                method_name,
                "onCreate"
                    | "onStart"
                    | "onRestart"
                    | "onResume"
                    | "onPause"
                    | "onStop"
                    | "onDestroy"
                    | "onNewIntent"
            )
        {
            return Ok(Value::Void);
        }
        let result = match (class_name, method_name) {
            ("Landroid/app/Activity;", "onCreate")
            | ("Landroid/app/Activity;", "onStart")
            | ("Landroid/app/Activity;", "onRestart")
            | ("Landroid/app/Activity;", "onResume")
            | ("Landroid/app/Activity;", "onPause")
            | ("Landroid/app/Activity;", "onStop")
            | ("Landroid/app/Activity;", "onDestroy")
            | ("Landroid/app/Activity;", "onSaveInstanceState")
            | ("Landroid/app/Activity;", "onRestoreInstanceState")
            | ("Landroid/app/Activity;", "onNewIntent")
            | ("Landroid/app/Activity;", "onWindowFocusChanged") => FrameworkResult::Void,
            ("Landroid/app/Activity;", "setContentView") => {
                let activity = object_arg(args, 0)?;
                let view = object_arg(args, 1)?;
                self.framework_call(FrameworkCall::SetContentView { activity, view })?
            }
            ("Landroid/app/Activity;", "finish") => {
                self.framework_call(FrameworkCall::FinishActivity {
                    activity: object_arg(args, 0)?,
                })?
            }
            ("Landroid/view/View;", "setId") => self.framework_call(FrameworkCall::SetViewId {
                view: object_arg(args, 0)?,
                id: int_arg(args, 1)?,
            })?,
            ("Landroid/widget/TextView;", "setText") => {
                self.framework_call(FrameworkCall::SetViewText {
                    view: object_arg(args, 0)?,
                    text: string_arg(args, 1)?,
                })?
            }
            ("Landroid/view/ViewGroup;", "addView") => {
                self.framework_call(FrameworkCall::AddView {
                    parent: object_arg(args, 0)?,
                    child: object_arg(args, 1)?,
                })?
            }
            ("Landroid/view/View;", "findViewById")
            | ("Landroid/app/Activity;", "findViewById") => {
                let id = int_arg(args, 1)?;
                let found = self
                    .framework
                    .views
                    .iter()
                    .find(|(_, view)| view.id == id)
                    .map(|(handle, _)| *handle)
                    .unwrap_or(0);
                FrameworkResult::Object(found)
            }
            ("Landroid/util/Log;", "d")
            | ("Landroid/util/Log;", "i")
            | ("Landroid/util/Log;", "w")
            | ("Landroid/util/Log;", "e")
            | ("Landroid/util/Log;", "v") => self.framework_call(FrameworkCall::Log {
                priority: 0,
                tag: string_arg(args, 0)?,
                message: string_arg(args, 1)?,
            })?,
            ("Landroid/widget/Toast;", "makeText") => {
                self.framework_call(FrameworkCall::Toast {
                    text: string_arg(args, 1)?,
                    duration: int_arg(args, 2)?,
                })?;
                FrameworkResult::Object(self.alloc_instance("Landroid/widget/Toast;"))
            }
            ("Landroid/widget/Toast;", "show") => FrameworkResult::Void,
            ("Landroid/content/Context;", "getString") => {
                FrameworkResult::String(self.framework_string(int_arg(args, 1)?)?)
            }
            ("Landroid/content/Context;", "getSystemService") => {
                self.framework_call(FrameworkCall::GetSystemService {
                    name: string_arg(args, 1)?,
                })?
            }
            ("Landroid/content/Context;", "getSharedPreferences") => {
                self.framework_call(FrameworkCall::GetSharedPreferences {
                    name: string_arg(args, 1)?,
                    mode: int_arg(args, 2)?,
                })?
            }
            ("Landroid/content/SharedPreferences;", "getString") => {
                self.framework_call(FrameworkCall::SharedPreferencesGetString {
                    prefs: object_arg(args, 0)?,
                    key: string_arg(args, 1)?,
                    default: string_arg(args, 2)?,
                })?
            }
            ("Landroid/content/SharedPreferences$Editor;", "putString") => {
                self.framework_call(FrameworkCall::SharedPreferencesPutString {
                    prefs: object_arg(args, 0)?,
                    key: string_arg(args, 1)?,
                    value: string_arg(args, 2)?,
                })?
            }
            ("Landroid/content/SharedPreferences$Editor;", "commit")
            | ("Landroid/content/SharedPreferences$Editor;", "apply") => {
                FrameworkResult::Bool(true)
            }
            ("Landroid/view/SurfaceHolder$Callback", "surfaceCreated") => {
                self.framework_call(FrameworkCall::SurfaceCreated {
                    surface: object_arg(args, 0)?,
                })?
            }
            ("Landroid/view/SurfaceHolder$Callback", "surfaceChanged") => {
                self.framework_call(FrameworkCall::SurfaceChanged {
                    surface: object_arg(args, 0)?,
                    format: int_arg(args, 1)?,
                    width: int_arg(args, 2)?,
                    height: int_arg(args, 3)?,
                })?
            }
            ("Landroid/view/SurfaceHolder$Callback", "surfaceDestroyed") => {
                self.framework_call(FrameworkCall::SurfaceDestroyed {
                    surface: object_arg(args, 0)?,
                })?
            }
            ("Landroid/media/AudioTrack;", "write") => {
                self.framework_call(FrameworkCall::AudioTrackWrite {
                    track: object_arg(args, 0)?,
                    samples: int_arg(args, 2).unwrap_or(0),
                })?
            }
            ("Landroid/media/MediaPlayer;", "prepare") => {
                self.framework_call(FrameworkCall::MediaPlayerPrepare {
                    player: object_arg(args, 0)?,
                })?
            }
            ("Landroid/media/MediaPlayer;", "start") => {
                self.framework_call(FrameworkCall::MediaPlayerStart {
                    player: object_arg(args, 0)?,
                })?
            }
            ("Landroid/media/MediaPlayer;", "stop") => {
                self.framework_call(FrameworkCall::MediaPlayerStop {
                    player: object_arg(args, 0)?,
                })?
            }
            ("Landroid/hardware/SensorManager;", "registerListener") => {
                self.framework_call(FrameworkCall::SensorRegister {
                    sensor: object_arg(args, 1)?,
                })?
            }
            ("Landroid/net/ConnectivityManager;", "getActiveNetworkInfo") => {
                FrameworkResult::Object(0)
            }
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

    fn framework_string(&self, id: i32) -> Result<String, VmError> {
        match self.framework.resources.get(id as u32) {
            Some(FrameworkValue::String(value)) => Ok(value.clone()),
            Some(FrameworkValue::Int(value)) => Ok(value.to_string()),
            _ => Err(self.error(0, 0, format!("resource string {id:#x} is unavailable"))),
        }
    }

    fn alloc(&mut self, object: HeapObject) -> ObjectId {
        let id = self.heap.len() as ObjectId;
        self.heap.push(object);
        id
    }

    fn method_code_by_index(&self, index: usize) -> Option<&CodeItem> {
        let method = self.dex.method_id(index)?;
        self.dex.method_code(&method.class_name, &method.name)
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
        instruction as usize >> 8 & 0x0f,
        (word & 0x0f) as usize,
        (word >> 4 & 0x0f) as usize,
        (word >> 8 & 0x0f) as usize,
        (word >> 12 & 0x0f) as usize,
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
        _ => Err(vm.error(pc, opcode, "value is not an object")),
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

fn int_arg(args: &[Value], index: usize) -> Result<i32, VmError> {
    match args.get(index) {
        Some(Value::Int(value)) => Ok(*value),
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
        Some(Value::Object(_)) => Ok(String::new()),
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
        return Err(VmError {
            pc: at,
            opcode,
            message: "branch target outside code".to_owned(),
        });
    }
    Ok(target as usize)
}
