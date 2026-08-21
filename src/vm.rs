use std::collections::HashMap;
use std::sync::OnceLock;

use crate::dalvik::{CodeItem, DexFile};
use crate::framework::{Framework, FrameworkCall, FrameworkResult};
use crate::Rgba8;

pub type ObjectId = u32;
static NANO_TIME_START: OnceLock<std::time::Instant> = OnceLock::new();

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
    frame_mode: bool,
    frame_aborted: bool,
    frame_steps: usize,
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
            frame_mode: false,
            frame_aborted: false,
            frame_steps: 0,
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

    fn alloc_bitmap(&mut self, width: i32, height: i32) -> ObjectId {
        let bitmap = self.alloc_instance("Landroid/graphics/Bitmap;");
        self.set_object_field(bitmap, "width", Value::Int(width.max(1)));
        self.set_object_field(bitmap, "height", Value::Int(height.max(1)));
        bitmap
    }

    fn alloc_reflective_array(&mut self, component: &str, dimensions: &[usize]) -> ObjectId {
        let length = dimensions.first().copied().unwrap_or(0);
        let values = if dimensions.len() > 1 {
            (0..length)
                .map(|_| Value::Object(self.alloc_reflective_array(component, &dimensions[1..])))
                .collect()
        } else {
            (0..length)
                .map(|_| default_value_for_type(component))
                .collect()
        };
        self.alloc(HeapObject::Array {
            component: component.to_owned(),
            values,
        })
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

    pub fn render_frame(&mut self, object: ObjectId, method_name: &str) -> Result<Value, VmError> {
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
        let args = vec![Value::Object(object)];
        self.frame_mode = true;
        self.frame_steps = 0;
        self.frame_aborted = false;
        let result = self.call_method(method_index, args);
        self.frame_mode = false;
        result
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
        if referenced.class_name == "Lcom/hyperkani/common/BaseObject;"
            && referenced.name == "render"
        {
            if let Some(HeapObject::Instance { class_name, .. }) = self.heap_object(object) {
                if class_name == "Lcom/hyperkani/common/GameObjectSprite;" {
                    return self
                        .dex
                        .methods
                        .iter()
                        .enumerate()
                        .find(|(_, method)| {
                            method.class_name == *class_name
                                && method.name == "render"
                                && method.prototype == referenced.prototype
                        })
                        .map(|(index, _)| index);
                }
            }
        }
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

    fn is_assignable(&self, actual: &str, expected: &str) -> bool {
        if actual == expected {
            return true;
        }
        let mut current = actual;
        let mut visited = std::collections::BTreeSet::new();
        while visited.insert(current.to_owned()) {
            let Some(super_class) = self
                .dex
                .find_class(current)
                .and_then(|class| class.super_class.as_deref())
            else {
                return false;
            };
            if super_class == expected {
                return true;
            }
            current = super_class;
        }
        false
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
            || method.class_name.starts_with("Lcom/hyperkani/common/")
            || method.class_name.starts_with("Lcom/mobclix/");
        if framework_class {
            if method.name == "<clinit>" {
                return Ok(Value::Void);
            }
            if method.class_name.starts_with("Lcom/mobclix/") {
                return Ok(Value::Void);
            }
            if method.class_name.starts_with("Lcom/badlogic/gdx/") {
                if method.name == "<clinit>"
                    || (method.name == "<init>"
                        && !matches!(
                            method.class_name.as_str(),
                            "Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;"
                                | "Lcom/badlogic/gdx/graphics/Texture;"
                                | "Lcom/badlogic/gdx/graphics/g2d/Sprite;"
                        ))
                {
                    return Ok(Value::Void);
                }
                return self.dispatch_gdx(&method.class_name, &method.name, &args);
            }
            if method.class_name.starts_with("Lcom/mobclix/") {
                return Ok(match method.name.as_str() {
                    "<init>" | "<clinit>" => Value::Void,
                    _ => Value::Void,
                });
            }
            if method.class_name.starts_with("Lcom/hyperkani/common/")
                && (method.name == "update"
                    || method.name == "dispose"
                    || method.name == "pause"
                    || method.name == "resume"
                    || method.name == "playSoundsFromThisFrame")
            {
                return Ok(Value::Void);
            }
            if method.name == "<init>"
                && (method.class_name == "Lcom/badlogic/gdx/math/Vector2;"
                    || method.class_name == "Lcom/badlogic/gdx/math/Vector3;"
                    || method.class_name == "Lcom/badlogic/gdx/math/Vector4;"
                    || method.class_name == "Lcom/badlogic/gdx/math/Matrix4;")
            {
                return Ok(Value::Void);
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
                if owner.starts_with("Lcom/badlogic/gdx/") {
                    return self.dispatch_gdx(&owner, &method.name, &args);
                }
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
                    if owner.starts_with("Lcom/badlogic/gdx/") {
                        return self.dispatch_gdx(&owner, &method.name, &args);
                    }
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
        let is_render_method =
            method.class_name == "Lcom/hyperkani/sliceice/Engine;" && method.name == "render";
        let previous_frame_mode = self.frame_mode;
        let previous_frame_aborted = self.frame_aborted;
        if is_render_method {
            self.frame_mode = true;
            self.frame_aborted = false;
        }
        self.call_depth += 1;
        let result = self.execute_code(&code, args);
        self.call_depth -= 1;
        if is_render_method {
            self.frame_mode = previous_frame_mode;
            self.frame_aborted = previous_frame_aborted;
        }
        result.map_err(|mut error| {
            error.message = format!(
                "{} in {}->{}",
                error.message, method.class_name, method.name
            );
            error
        })
    }

    fn execute_code(&mut self, code: &CodeItem, args: Vec<Value>) -> Result<Value, VmError> {
        if self.frame_mode && self.frame_aborted {
            return Ok(Value::Void);
        }
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
            if self.frame_mode {
                self.frame_steps += 1;
                if self.frame_steps > self.config.max_steps {
                    return Ok(Value::Void);
                }
            }
            if !self.frame_mode && self.executed_steps > self.config.max_steps {
                return Err(self.error(pc, 0, "instruction limit exceeded"));
            }
            let instruction = code.instructions[pc];
            let opcode = (instruction & 0xff) as u8;
            match opcode {
                0x00 => pc += 1,
                0x01 | 0x07 => {
                    let (dest, source) = two_registers(instruction);
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 1;
                }
                0x02 | 0x08 => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    let source = code_word(code, pc + 1, pc, opcode)? as usize;
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 2;
                }
                0x03 | 0x09 => {
                    let dest = code_word(code, pc + 1, pc, opcode)? as usize;
                    let source = code_word(code, pc + 2, pc, opcode)? as usize;
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 3;
                }
                0x04 => {
                    let (dest, source) = two_registers(instruction);
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_wide_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 1;
                }
                0x05 => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    let source = code_word(code, pc + 1, pc, opcode)? as usize;
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_wide_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 2;
                }
                0x06 => {
                    let dest = code_word(code, pc + 1, pc, opcode)? as usize;
                    let source = code_word(code, pc + 2, pc, opcode)? as usize;
                    let value = get_register(&registers, source, pc, opcode)?;
                    set_wide_register(&mut registers, dest, value, self, pc, opcode)?;
                    pc += 3;
                }
                0x0a | 0x0c => {
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
                0x0b => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    set_wide_register(
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
                0x0f => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    return get_register(&registers, register, pc, opcode);
                }
                0x10 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    return read_wide_register(&registers, register, pc, opcode);
                }
                0x11 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    return get_register(&registers, register, pc, opcode);
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
                0x7b => {
                    let (dest, source) = two_registers(instruction);
                    let value = -as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x7c => {
                    let (dest, source) = two_registers(instruction);
                    let value = !as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x7d => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        -as_long(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    set_wide_register(&mut registers, dest, Value::Long(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x7e => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        !as_long(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    set_wide_register(&mut registers, dest, Value::Long(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x7f => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        -as_float(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    set_register(&mut registers, dest, Value::Float(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x80 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        -as_double(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    set_wide_register(
                        &mut registers,
                        dest,
                        Value::Double(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 1;
                }
                0x81 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)? as i64;
                    set_wide_register(&mut registers, dest, Value::Long(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x82 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)? as f32;
                    set_register(&mut registers, dest, Value::Float(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x83 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)? as f64;
                    set_wide_register(
                        &mut registers,
                        dest,
                        Value::Double(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 1;
                }
                0x84 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_long(get_register(&registers, source, pc, opcode)?, pc, opcode)? as i32;
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x85 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_long(get_register(&registers, source, pc, opcode)?, pc, opcode)? as f32;
                    set_register(&mut registers, dest, Value::Float(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x86 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_long(get_register(&registers, source, pc, opcode)?, pc, opcode)? as f64;
                    set_wide_register(
                        &mut registers,
                        dest,
                        Value::Double(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 1;
                }
                0x87 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_float(get_register(&registers, source, pc, opcode)?, pc, opcode)? as i32;
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x88 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_float(get_register(&registers, source, pc, opcode)?, pc, opcode)? as i64;
                    set_wide_register(&mut registers, dest, Value::Long(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x89 => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_float(get_register(&registers, source, pc, opcode)?, pc, opcode)? as f64;
                    set_wide_register(
                        &mut registers,
                        dest,
                        Value::Double(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 1;
                }
                0x8a => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_double(get_register(&registers, source, pc, opcode)?, pc, opcode)?
                            as i32;
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x8b => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_double(get_register(&registers, source, pc, opcode)?, pc, opcode)?
                            as i64;
                    set_wide_register(&mut registers, dest, Value::Long(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x8c => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_double(get_register(&registers, source, pc, opcode)?, pc, opcode)?
                            as f32;
                    set_register(&mut registers, dest, Value::Float(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x8d => {
                    let (dest, source) = two_registers(instruction);
                    let value = as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?
                        as i8 as i32;
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x8e => {
                    let (dest, source) = two_registers(instruction);
                    let value =
                        as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)? & 0xffff;
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x8f => {
                    let (dest, source) = two_registers(instruction);
                    let value = as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?
                        as i16 as i32;
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0x16 => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let value = code_word(code, pc + 1, pc, opcode)? as i16 as i64;
                    set_wide_register(
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
                    set_wide_register(
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
                    set_wide_register(
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
                    set_wide_register(
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
                    let _type_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let _value = get_register(&registers, register, pc, opcode)?;
                    pc += 2;
                }
                0x20 => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    let source = ((instruction >> 12) & 0x0f) as usize;
                    let type_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let expected = self.dex.types.get(type_index).map(String::as_str);
                    let result = match get_register(&registers, source, pc, opcode)? {
                        Value::Object(id) => match (self.heap_object(id), expected) {
                            (Some(HeapObject::Instance { class_name, .. }), Some(expected)) => {
                                self.is_assignable(class_name, expected)
                            }
                            (Some(HeapObject::Instance { .. }), None) => true,
                            _ => false,
                        },
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
                0x24 | 0x25 => {
                    let type_index = code_word(code, pc + 1, pc, opcode)? as usize;
                    let component = self.dex.types.get(type_index).cloned().ok_or_else(|| {
                        self.error(pc, opcode, "filled-new-array type index is invalid")
                    })?;
                    let args = if opcode == 0x24 {
                        invoke_args(
                            &registers,
                            instruction,
                            code_word(code, pc + 2, pc, opcode)?,
                            pc,
                            opcode,
                        )?
                    } else {
                        let count = ((instruction >> 8) & 0xff) as usize;
                        let first = code_word(code, pc + 2, pc, opcode)? as usize;
                        (0..count)
                            .map(|offset| get_register(&registers, first + offset, pc, opcode))
                            .collect::<Result<Vec<_>, _>>()?
                    };
                    let array = self.alloc(HeapObject::Array {
                        component,
                        values: args,
                    });
                    pending_result = Value::Object(array);
                    pc += 3;
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
                0x28 => {
                    let offset = (instruction >> 8) as u8 as i8 as i32;
                    pc = branch_target(pc, offset, code.instructions.len(), pc, opcode)?;
                }
                0x29 => {
                    let offset = code_word(code, pc + 1, pc, opcode)? as i16 as i32;
                    pc = branch_target(pc, offset, code.instructions.len(), pc, opcode)?;
                }
                0x2a => {
                    let offset = code_word(code, pc + 1, pc, opcode)? as i16 as i32;
                    pc = branch_target(pc, offset, code.instructions.len(), pc, opcode)?;
                }
                0x2d..=0x31 => {
                    let (dest, left_reg, right_reg) =
                        three_registers(instruction, code_word(code, pc + 1, pc, opcode)?);
                    let left = get_register(&registers, left_reg, pc, opcode)?;
                    let right = get_register(&registers, right_reg, pc, opcode)?;
                    let value = match opcode {
                        0x2d => compare_float(
                            as_float(left, pc, opcode)?,
                            as_float(right, pc, opcode)?,
                            false,
                        ),
                        0x2e => compare_float(
                            as_float(left, pc, opcode)?,
                            as_float(right, pc, opcode)?,
                            true,
                        ),
                        0x2f => compare_double(
                            as_double(left, pc, opcode)?,
                            as_double(right, pc, opcode)?,
                            false,
                        ),
                        0x30 => compare_double(
                            as_double(left, pc, opcode)?,
                            as_double(right, pc, opcode)?,
                            true,
                        ),
                        _ => compare_long(as_long(left, pc, opcode)?, as_long(right, pc, opcode)?),
                    };
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 2;
                }
                0x90..=0x9a => {
                    let (dest, left_reg, right_reg) =
                        three_registers(instruction, code_word(code, pc + 1, pc, opcode)?);
                    let left = as_int(get_register(&registers, left_reg, pc, opcode)?, pc, opcode)?;
                    let right = get_register(&registers, right_reg, pc, opcode)?;
                    let right_int = as_int(right, pc, opcode)?;
                    let value = match opcode {
                        0x90 => left.wrapping_add(right_int),
                        0x91 => left.wrapping_sub(right_int),
                        0x92 => left.wrapping_mul(right_int),
                        0x93 => {
                            if right_int == 0 {
                                return Err(self.error(pc, opcode, "integer division by zero"));
                            }
                            left.wrapping_div(right_int)
                        }
                        0x94 => {
                            if right_int == 0 {
                                return Err(self.error(pc, opcode, "integer remainder by zero"));
                            }
                            left.wrapping_rem(right_int)
                        }
                        0x95 => left & right_int,
                        0x96 => left | right_int,
                        0x97 => left ^ right_int,
                        0x98 => left.wrapping_shl((right_int & 0x1f) as u32),
                        0x99 => left >> (right_int & 0x1f),
                        _ => ((left as u32) >> (right_int & 0x1f)) as i32,
                    };
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 2;
                }
                0x9b..=0xa5 => {
                    let (dest, left_reg, right_reg) =
                        three_registers(instruction, code_word(code, pc + 1, pc, opcode)?);
                    let left =
                        as_long(get_register(&registers, left_reg, pc, opcode)?, pc, opcode)?;
                    let right_value = get_register(&registers, right_reg, pc, opcode)?;
                    let value = match opcode {
                        0x9b => left.wrapping_add(as_long(right_value, pc, opcode)?),
                        0x9c => left.wrapping_sub(as_long(right_value, pc, opcode)?),
                        0x9d => left.wrapping_mul(as_long(right_value, pc, opcode)?),
                        0x9e => {
                            let right = as_long(right_value, pc, opcode)?;
                            if right == 0 {
                                return Err(self.error(pc, opcode, "long division by zero"));
                            }
                            left.wrapping_div(right)
                        }
                        0x9f => {
                            let right = as_long(right_value, pc, opcode)?;
                            if right == 0 {
                                return Err(self.error(pc, opcode, "long remainder by zero"));
                            }
                            left.wrapping_rem(right)
                        }
                        0xa0 => left & as_long(right_value, pc, opcode)?,
                        0xa1 => left | as_long(right_value, pc, opcode)?,
                        0xa2 => left ^ as_long(right_value, pc, opcode)?,
                        0xa3 => left.wrapping_shl((as_int(right_value, pc, opcode)? & 0x3f) as u32),
                        0xa4 => left >> (as_int(right_value, pc, opcode)? & 0x3f),
                        _ => ((left as u64) >> (as_int(right_value, pc, opcode)? & 0x3f)) as i64,
                    };
                    set_wide_register(&mut registers, dest, Value::Long(value), self, pc, opcode)?;
                    pc += 2;
                }
                0xa6..=0xaa => {
                    let (dest, left_reg, right_reg) =
                        three_registers(instruction, code_word(code, pc + 1, pc, opcode)?);
                    let left =
                        as_float(get_register(&registers, left_reg, pc, opcode)?, pc, opcode)?;
                    let right =
                        as_float(get_register(&registers, right_reg, pc, opcode)?, pc, opcode)?;
                    let value = match opcode {
                        0xa6 => left + right,
                        0xa7 => left - right,
                        0xa8 => left * right,
                        0xa9 => left / right,
                        _ => left % right,
                    };
                    set_register(&mut registers, dest, Value::Float(value), self, pc, opcode)?;
                    pc += 2;
                }
                0xab..=0xaf => {
                    let (dest, left_reg, right_reg) =
                        three_registers(instruction, code_word(code, pc + 1, pc, opcode)?);
                    let left =
                        as_double(get_register(&registers, left_reg, pc, opcode)?, pc, opcode)?;
                    let right =
                        as_double(get_register(&registers, right_reg, pc, opcode)?, pc, opcode)?;
                    let value = match opcode {
                        0xab => left + right,
                        0xac => left - right,
                        0xad => left * right,
                        0xae => left / right,
                        _ => left % right,
                    };
                    set_wide_register(
                        &mut registers,
                        dest,
                        Value::Double(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 2;
                }
                0xb0..=0xb7 => {
                    let (dest, source) = two_registers(instruction);
                    let left = as_int(get_register(&registers, dest, pc, opcode)?, pc, opcode)?;
                    let right = as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    let value = match opcode {
                        0xb0 => left.wrapping_add(right),
                        0xb1 => left.wrapping_sub(right),
                        0xb2 => left.wrapping_mul(right),
                        0xb3 => {
                            if right == 0 {
                                return Err(self.error(pc, opcode, "integer division by zero"));
                            }
                            left.wrapping_div(right)
                        }
                        0xb4 => {
                            if right == 0 {
                                return Err(self.error(pc, opcode, "integer remainder by zero"));
                            }
                            left.wrapping_rem(right)
                        }
                        0xb5 => left & right,
                        0xb6 => left | right,
                        _ => left ^ right,
                    };
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0xb8..=0xba => {
                    let (dest, source) = two_registers(instruction);
                    let left = as_int(get_register(&registers, dest, pc, opcode)?, pc, opcode)?;
                    let right = as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    let shift = (right & 0x1f) as u32;
                    let value = match opcode {
                        0xb8 => left.wrapping_shl(shift),
                        0xb9 => left >> shift,
                        _ => ((left as u32) >> shift) as i32,
                    };
                    set_register(&mut registers, dest, Value::Int(value), self, pc, opcode)?;
                    pc += 1;
                }
                0xbb..=0xbf => {
                    let (dest, source) = two_registers(instruction);
                    let left = as_long(get_register(&registers, dest, pc, opcode)?, pc, opcode)?;
                    let right = as_long(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    let value = match opcode {
                        0xbb => left.wrapping_add(right),
                        0xbc => left.wrapping_sub(right),
                        0xbd => left.wrapping_mul(right),
                        0xbe => {
                            if right == 0 {
                                return Err(self.error(pc, opcode, "long division by zero"));
                            }
                            left.wrapping_div(right)
                        }
                        _ => {
                            if right == 0 {
                                return Err(self.error(pc, opcode, "long remainder by zero"));
                            }
                            left.wrapping_rem(right)
                        }
                    };
                    set_wide_register(&mut registers, dest, Value::Long(value), self, pc, opcode)?;
                    pc += 1;
                }
                0xc0..=0xc2 => {
                    let (dest, source) = two_registers(instruction);
                    let left = as_long(get_register(&registers, dest, pc, opcode)?, pc, opcode)?;
                    let right = as_long(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    let value = match opcode {
                        0xc0 => left & right,
                        0xc1 => left | right,
                        _ => left ^ right,
                    };
                    set_wide_register(&mut registers, dest, Value::Long(value), self, pc, opcode)?;
                    pc += 1;
                }
                0xc3..=0xc5 => {
                    let (dest, source) = two_registers(instruction);
                    let left = as_long(get_register(&registers, dest, pc, opcode)?, pc, opcode)?;
                    let shift = (as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?
                        & 0x3f) as u32;
                    let value = match opcode {
                        0xc3 => left.wrapping_shl(shift),
                        0xc4 => left >> shift,
                        _ => ((left as u64) >> shift) as i64,
                    };
                    set_wide_register(&mut registers, dest, Value::Long(value), self, pc, opcode)?;
                    pc += 1;
                }
                0xc6..=0xca => {
                    let (dest, source) = two_registers(instruction);
                    let left_value = get_register(&registers, dest, pc, opcode)?;
                    let right_value = get_register(&registers, source, pc, opcode)?;
                    let left = as_float(left_value.clone(), pc, opcode).map_err(|error| {
                        self.error(
                            error.pc,
                            error.opcode,
                            format!("float 2addr left v{dest}={left_value:?}: {}", error.message),
                        )
                    })?;
                    let right = as_float(right_value.clone(), pc, opcode).map_err(|error| {
                        self.error(
                            error.pc,
                            error.opcode,
                            format!(
                                "float 2addr right v{source}={right_value:?}: {}",
                                error.message
                            ),
                        )
                    })?;
                    let value = match opcode {
                        0xc6 => left + right,
                        0xc7 => left - right,
                        0xc8 => left * right,
                        0xc9 => left / right,
                        _ => left % right,
                    };
                    set_register(&mut registers, dest, Value::Float(value), self, pc, opcode)?;
                    pc += 1;
                }
                0xcb..=0xcf => {
                    let (dest, source) = two_registers(instruction);
                    let left = as_double(
                        read_wide_register(&registers, dest, pc, opcode)?,
                        pc,
                        opcode,
                    )?;
                    let right = as_double(
                        read_wide_register(&registers, source, pc, opcode)?,
                        pc,
                        opcode,
                    )?;
                    let value = match opcode {
                        0xcb => left + right,
                        0xcc => left - right,
                        0xcd => left * right,
                        0xce => left / right,
                        _ => left % right,
                    };
                    set_wide_register(
                        &mut registers,
                        dest,
                        Value::Double(value),
                        self,
                        pc,
                        opcode,
                    )?;
                    pc += 1;
                }
                0x32..=0x37 => {
                    let (left, right) = two_registers(instruction);
                    let offset = code_word(code, pc + 1, pc, opcode)? as i16 as i32;
                    let left_value = get_register(&registers, left, pc, opcode)?;
                    let right_value = get_register(&registers, right, pc, opcode)?;
                    let take = match opcode {
                        0x32 => values_equal(&left_value, &right_value),
                        0x33 => !values_equal(&left_value, &right_value),
                        0x34 => compare_values(&left_value, &right_value, pc, opcode)? < 0,
                        0x35 => compare_values(&left_value, &right_value, pc, opcode)? >= 0,
                        0x36 => compare_values(&left_value, &right_value, pc, opcode)? > 0,
                        _ => compare_values(&left_value, &right_value, pc, opcode)? <= 0,
                    };
                    if take {
                        pc = branch_target(pc, offset, code.instructions.len(), pc, opcode)?;
                    } else {
                        pc += 2;
                    }
                }
                0xd0..=0xd7 => {
                    let dest = ((instruction >> 8) & 0x0f) as usize;
                    let source = ((instruction >> 12) & 0x0f) as usize;
                    let literal = code_word(code, pc + 1, pc, opcode)? as i16 as i32;
                    let value = as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    let result = match opcode {
                        0xd0 => value.wrapping_add(literal),
                        0xd1 => literal.wrapping_sub(value),
                        0xd2 => value.wrapping_mul(literal),
                        0xd3 => {
                            if literal == 0 {
                                return Err(self.error(pc, opcode, "integer division by zero"));
                            }
                            value.wrapping_div(literal)
                        }
                        0xd4 => {
                            if literal == 0 {
                                return Err(self.error(pc, opcode, "integer remainder by zero"));
                            }
                            value.wrapping_rem(literal)
                        }
                        0xd5 => value & literal,
                        0xd6 => value | literal,
                        _ => value ^ literal,
                    };
                    set_register(&mut registers, dest, Value::Int(result), self, pc, opcode)?;
                    pc += 2;
                }
                0xd8..=0xdf => {
                    let dest = ((instruction >> 8) & 0xff) as usize;
                    let literal_word = code_word(code, pc + 1, pc, opcode)?;
                    let source = (literal_word & 0xff) as usize;
                    let literal = (literal_word >> 8) as u8 as i8 as i32;
                    let value = as_int(get_register(&registers, source, pc, opcode)?, pc, opcode)?;
                    let result = match opcode {
                        0xd8 => value.wrapping_add(literal),
                        0xd9 => literal.wrapping_sub(value),
                        0xda => value.wrapping_mul(literal),
                        0xdb => {
                            if literal == 0 {
                                return Err(self.error(pc, opcode, "integer division by zero"));
                            }
                            value.wrapping_div(literal)
                        }
                        0xdc => {
                            if literal == 0 {
                                return Err(self.error(pc, opcode, "integer remainder by zero"));
                            }
                            value.wrapping_rem(literal)
                        }
                        0xdd => value & literal,
                        0xde => value | literal,
                        _ => value ^ literal,
                    };
                    set_register(&mut registers, dest, Value::Int(result), self, pc, opcode)?;
                    pc += 2;
                }
                0x38..=0x3d => {
                    let register = ((instruction >> 8) & 0xff) as usize;
                    let offset = code_word(code, pc + 1, pc, opcode)? as i16 as i32;
                    let value = get_register(&registers, register, pc, opcode)?;
                    let zero = match &value {
                        Value::Null | Value::Void => true,
                        Value::Int(value) => *value == 0,
                        Value::Long(value) => *value == 0,
                        Value::Float(value) => *value == 0.0,
                        Value::Double(value) => *value == 0.0,
                        Value::Object(_) | Value::String(_) => false,
                    };
                    let take = match opcode {
                        0x38 => zero,
                        0x39 => !zero,
                        0x3a => as_int(value.clone(), pc, opcode)? < 0,
                        0x3b => as_int(value.clone(), pc, opcode)? >= 0,
                        0x3c => as_int(value.clone(), pc, opcode)? > 0,
                        0x3d => as_int(value.clone(), pc, opcode)? <= 0,
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
                    if opcode == 0x45 {
                        set_wide_register(&mut registers, dest, value, self, pc, opcode)?;
                    } else {
                        set_register(&mut registers, dest, value, self, pc, opcode)?;
                    }
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
                        let object = match get_register(&registers, object_register, pc, opcode)? {
                            Value::Object(id) => id,
                            Value::Null => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    format!("null instance field target: field={field_key} v{object_register} registers={registers:?}"),
                                ));
                            }
                            value => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    format!("instance field target is not an object: field={field_key} v{object_register} value={value:?}"),
                                ));
                            }
                        };
                        let existing = match self.heap_object(object) {
                            Some(HeapObject::Instance {
                                class_name: object_class,
                                fields,
                            }) => {
                                let is_main_layer = self
                                    .dex
                                    .field_id(field_index as usize)
                                    .is_some_and(|field| field.name == "mMainLayer");
                                if is_main_layer {
                                    fields.get(&field_key).cloned().or_else(|| {
                                        fields
                                            .iter()
                                            .find(|(key, _)| {
                                                key.starts_with(&format!(
                                                    "{object_class}->mMainLayer:"
                                                ))
                                            })
                                            .or_else(|| {
                                                fields.iter().find(|(key, _)| {
                                                    **key != field_key
                                                        && key.contains("->mMainLayer:")
                                                })
                                            })
                                            .map(|(_, value)| value.clone())
                                    })
                                } else {
                                    fields.get(&field_key).cloned().or_else(|| {
                                        self.dex.field_id(field_index as usize).and_then(|field| {
                                            let suffix =
                                                format!("->{}:{}", field.name, field.type_name);
                                            fields.iter().find_map(|(key, value)| {
                                                key.ends_with(&suffix).then(|| value.clone())
                                            })
                                        })
                                    })
                                }
                            }
                            Some(HeapObject::Class(class_name)) => {
                                let class_name = class_name.clone();
                                self.heap[object as usize] = HeapObject::Instance {
                                    class_name,
                                    fields: HashMap::new(),
                                };
                                None
                            }
                            Some(_) => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    "instance field target is not an object",
                                ));
                            }
                            None => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    format!("instance field target object id is invalid: field={field_key} object={object}"),
                                ));
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
                        } else if matches!(
                            self.dex.field_id(field_index as usize),
                            Some(field) if matches!(
                                field.type_name.as_str(),
                                "Ljava/util/ArrayList;"
                            )
                        ) {
                            let collection = self.alloc(HeapObject::Collection(Vec::new()));
                            self.set_object_field(object, &field_key, Value::Object(collection));
                            Value::Object(collection)
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
                        let object = match get_register(&registers, object_register, pc, opcode)? {
                            Value::Object(id) => id,
                            Value::Null => {
                                let field = self.dex.field_id(field_index as usize);
                                if field.is_some_and(|field| {
                                    matches!(field.type_name.as_str(), "I" | "Z" | "B" | "S" | "C" | "F" | "J" | "D")
                                }) {
                                    pc += 2;
                                    continue;
                                }
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    format!("null instance field target: field={field_key} registers={registers:?}"),
                                ));
                            }
                            value => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    format!("instance field target is not an object: field={field_key} value={value:?}"),
                                ))
                            }
                        };
                        let value = get_register(&registers, value_register, pc, opcode)?.clone();
                        match self.heap.get_mut(object as usize) {
                            Some(HeapObject::Instance { fields, .. }) => {
                                fields.insert(field_key, value);
                            }
                            Some(HeapObject::Class(class_name)) => {
                                let class_name = class_name.clone();
                                self.heap[object as usize] = HeapObject::Instance {
                                    class_name,
                                    fields: HashMap::new(),
                                };
                                if let Some(HeapObject::Instance { fields, .. }) =
                                    self.heap.get_mut(object as usize)
                                {
                                    fields.insert(field_key, value);
                                }
                            }
                            Some(_) => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    "instance field target is not an instance",
                                ));
                            }
                            None => {
                                return Err(self.error(
                                    pc,
                                    opcode,
                                    format!("instance field target object id is invalid: field={field_key} object={object}"),
                                ));
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
                    let field = self.dex.field_id(field_index as usize).cloned();
                    if opcode <= 0x66 {
                        if let Some(field) = &field {
                            self.ensure_class_initialized(&field.class_name)?;
                        }
                        let value = self
                            .static_fields
                            .get(&field_key)
                            .cloned()
                            .or_else(|| {
                                field.as_ref().map(|field| match field.name.as_str() {
                                    "width" => {
                                        Value::Float(self.framework.surface_size.0.max(320) as f32)
                                    }
                                    "height" => {
                                        Value::Float(self.framework.surface_size.1.max(480) as f32)
                                    }
                                    "RELEASE" => Value::String("1.6".to_owned()),
                                    "DEVICE" => Value::String("donuthle".to_owned()),
                                    "MODEL" => Value::String("DonutHLE Linux".to_owned()),
                                    _ => default_value_for_type(&field.type_name),
                                })
                            })
                            .unwrap_or(Value::Null);
                        if opcode == 0x61 || opcode == 0x65 {
                            set_wide_register(&mut registers, register, value, self, pc, opcode)?;
                        } else {
                            set_register(&mut registers, register, value, self, pc, opcode)?;
                        }
                    } else {
                        let value = if opcode == 0x68 || opcode == 0x6c {
                            read_wide_register(&registers, register, pc, opcode)?
                        } else {
                            get_register(&registers, register, pc, opcode)?
                        };
                        self.static_fields.insert(field_key, value);
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
        if class_name == "Ljava/util/Collections;" && method_name == "sort" {
            return Ok(Value::Void);
        }
        if class_name == "Ljava/io/PrintStream;" {
            return match method_name {
                "print" | "println" | "flush" | "close" => Ok(Value::Void),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Ljava/lang/Thread;" && method_name == "sleep" {
            let milliseconds = int_arg(args, 0)?;
            if milliseconds >= 0 {
                let delay = if self.frame_mode {
                    milliseconds.max(1)
                } else {
                    milliseconds
                };
                std::thread::sleep(std::time::Duration::from_millis(delay as u64));
            }
            return Ok(Value::Void);
        }
        if class_name == "Ljava/lang/Thread;" {
            return match method_name {
                "currentThread" => Ok(Value::Object(self.alloc_instance("Ljava/lang/Thread;"))),
                "setPriority" | "yield" => Ok(Value::Void),
                "getPriority" => Ok(Value::Int(5)),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/os/Looper;" {
            return match method_name {
                "getThread" => Ok(Value::Object(self.alloc_instance("Ljava/lang/Thread;"))),
                "loop" | "prepare" => Ok(Value::Void),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Ljava/lang/System;" && method_name == "nanoTime" {
            let clock = NANO_TIME_START.get_or_init(std::time::Instant::now);
            return Ok(Value::Long(clock.elapsed().as_nanos() as i64));
        }
        if class_name == "Ljava/util/Locale;" {
            return match method_name {
                "getDefault" => Ok(Value::Object(self.alloc_instance("Ljava/util/Locale;"))),
                "getCountry" => Ok(Value::String("EN".to_owned())),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/os/Environment;" && method_name == "getExternalStorageDirectory"
        {
            return Ok(Value::Object(self.alloc_instance("Ljava/io/File;")));
        }
        if class_name == "Landroid/content/Context;" || class_name.starts_with("Landroid/app/") {
            match method_name {
                "getApplicationContext" => return Ok(args.first().cloned().unwrap_or(Value::Null)),
                "getResources" => {
                    return Ok(Value::Object(
                        self.alloc_instance("Landroid/content/res/Resources;"),
                    ))
                }
                "getSystemService" => {
                    let name = self.string_arg(args, 1).unwrap_or_default();
                    let service_class = match name.as_str() {
                        "sensor" => "Landroid/hardware/SensorManager;",
                        "vibrator" => "Landroid/os/Vibrator;",
                        "audio" => "Landroid/media/AudioManager;",
                        _ => "Landroid/content/Context;",
                    };
                    let object = self
                        .framework
                        .system_services
                        .get(&name)
                        .copied()
                        .unwrap_or_else(|| {
                            let id = self.alloc_instance(service_class);
                            self.framework.system_services.insert(name, id);
                            id
                        });
                    return Ok(Value::Object(object));
                }
                "getSharedPreferences" => {
                    return Ok(Value::Object(
                        self.alloc_instance("Landroid/content/SharedPreferences;"),
                    ))
                }
                "registerReceiver" => return Ok(Value::Null),
                "getMainLooper" => {
                    return Ok(Value::Object(self.alloc_instance("Landroid/os/Looper;")))
                }
                "setContentView" => return Ok(Value::Void),
                "findViewById" => {
                    let id = int_arg(args, 1)?;
                    let class_name = match id {
                        2131492892 => "Landroid/widget/ViewFlipper;",
                        2131492894 => "Lcom/mobclix/android/sdk/MobclixMMABannerXLAdView;",
                        _ => "Landroid/view/View;",
                    };
                    return Ok(Value::Object(self.alloc_instance(class_name)));
                }
                "setRequestedOrientation" | "requestWindowFeature" => return Ok(Value::Int(1)),
                "getWindow" => {
                    return Ok(Value::Object(self.alloc_instance("Landroid/view/Window;")))
                }
                "getWindowManager" => {
                    return Ok(Value::Object(
                        self.alloc_instance("Landroid/view/WindowManager;"),
                    ))
                }
                _ => {}
            }
        }
        if class_name == "Landroid/content/SharedPreferences;" {
            return match method_name {
                "getBoolean" => Ok(Value::Int(int_arg(args, 2)?)),
                "getInt" => Ok(Value::Int(int_arg(args, 2)?)),
                "getLong" => Ok(Value::Long(int_arg(args, 2)? as i64)),
                "getFloat" => Ok(Value::Float(float_arg(args, 2)?)),
                "getString" => Ok(args.get(2).cloned().unwrap_or(Value::String(String::new()))),
                "edit" => Ok(Value::Object(
                    self.alloc_instance("Landroid/content/SharedPreferences$Editor;"),
                )),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/content/res/Resources;" {
            return match method_name {
                "getDisplayMetrics" => Ok(Value::Object(
                    self.alloc_instance("Landroid/util/DisplayMetrics;"),
                )),
                "getStringArray" => {
                    let values = (0..3)
                        .map(|_| Value::Object(self.alloc_string(String::new())))
                        .collect();
                    Ok(Value::Object(self.alloc(HeapObject::Array {
                        component: "Ljava/lang/String;".to_owned(),
                        values,
                    })))
                }
                "getString" => Ok(Value::String(String::new())),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/graphics/BitmapFactory;" {
            return match method_name {
                "decodeResource" | "decodeStream" | "decodeFile" => {
                    Ok(Value::Object(self.alloc_bitmap(320, 480)))
                }
                _ => Ok(Value::Null),
            };
        }
        if class_name == "Landroid/graphics/Bitmap;" {
            return match method_name {
                "getWidth" => Ok(Value::Int(
                    object_arg(args, 0)
                        .ok()
                        .and_then(|id| self.object_field_int(id, "width"))
                        .unwrap_or(320),
                )),
                "getHeight" => Ok(Value::Int(
                    object_arg(args, 0)
                        .ok()
                        .and_then(|id| self.object_field_int(id, "height"))
                        .unwrap_or(480),
                )),
                "recycle" | "eraseColor" => Ok(Value::Void),
                "isRecycled" => Ok(Value::Int(0)),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/view/animation/AnimationUtils;" {
            if method_name == "loadAnimation" {
                return Ok(Value::Object(
                    self.alloc_instance("Landroid/view/animation/Animation;"),
                ));
            }
            return Ok(Value::Null);
        }
        if class_name == "Landroid/view/animation/Animation;" {
            return Ok(Value::Void);
        }
        if class_name == "Landroid/content/SharedPreferences$Editor;" {
            return match method_name {
                "putBoolean" | "putInt" | "putLong" | "putFloat" | "putString" => {
                    Ok(Value::Object(object_arg(args, 0)?))
                }
                "commit" => Ok(Value::Int(1)),
                "apply" => Ok(Value::Void),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Ljava/lang/System;" && method_name == "arraycopy" {
            let source = object_arg(args, 0)?;
            let source_pos = int_arg(args, 1)? as usize;
            let destination = object_arg(args, 2)?;
            let destination_pos = int_arg(args, 3)? as usize;
            let length = int_arg(args, 4)?.max(0) as usize;
            let values = match self.heap_object(source) {
                Some(HeapObject::Array { values, .. }) => values.clone(),
                _ => return Err(self.error(0, 0, "System.arraycopy source is not an array")),
            };
            let destination_values = match self.heap.get_mut(destination as usize) {
                Some(HeapObject::Array { values, .. }) => values,
                _ => return Err(self.error(0, 0, "System.arraycopy destination is not an array")),
            };
            if source_pos.saturating_add(length) > values.len()
                || destination_pos.saturating_add(length) > destination_values.len()
            {
                return Err(self.error(0, 0, "System.arraycopy range is outside an array"));
            }
            destination_values[destination_pos..destination_pos + length]
                .clone_from_slice(&values[source_pos..source_pos + length]);
            return Ok(Value::Void);
        }
        if class_name == "Ljava/lang/Math;" {
            return match method_name {
                "round" => Ok(Value::Int(match args.first() {
                    Some(Value::Float(value)) => value.round() as i32,
                    Some(Value::Double(value)) => value.round() as i32,
                    Some(Value::Int(value)) => *value,
                    Some(Value::Long(value)) => *value as i32,
                    _ => 0,
                })),
                "abs" => Ok(match args.first() {
                    Some(Value::Float(value)) => Value::Float(value.abs()),
                    Some(Value::Double(value)) => Value::Double(value.abs()),
                    Some(Value::Long(value)) => Value::Long(value.abs()),
                    Some(Value::Int(value)) => Value::Int(value.abs()),
                    _ => Value::Int(0),
                }),
                "sqrt" => Ok(match args.first() {
                    Some(Value::Float(value)) => Value::Float(value.sqrt()),
                    Some(Value::Double(value)) => Value::Double(value.sqrt()),
                    Some(Value::Long(value)) => Value::Double((*value as f64).sqrt()),
                    Some(Value::Int(value)) => Value::Double((*value as f64).sqrt()),
                    _ => Value::Double(0.0),
                }),
                "sin" => Ok(match args.first() {
                    Some(Value::Float(value)) => Value::Float(value.sin()),
                    Some(Value::Double(value)) => Value::Double(value.sin()),
                    Some(Value::Long(value)) => Value::Double((*value as f64).sin()),
                    Some(Value::Int(value)) => Value::Double((*value as f64).sin()),
                    _ => Value::Double(0.0),
                }),
                "cos" => Ok(match args.first() {
                    Some(Value::Float(value)) => Value::Float(value.cos()),
                    Some(Value::Double(value)) => Value::Double(value.cos()),
                    Some(Value::Long(value)) => Value::Double((*value as f64).cos()),
                    Some(Value::Int(value)) => Value::Double((*value as f64).cos()),
                    _ => Value::Double(0.0),
                }),
                "min" | "max" => {
                    let left = args.first().cloned().unwrap_or(Value::Int(0));
                    let right = args.get(1).cloned().unwrap_or(Value::Int(0));
                    Ok(match (left, right) {
                        (Value::Float(left), Value::Float(right)) => {
                            if method_name == "min" {
                                Value::Float(left.min(right))
                            } else {
                                Value::Float(left.max(right))
                            }
                        }
                        (Value::Double(left), Value::Double(right)) => {
                            if method_name == "min" {
                                Value::Double(left.min(right))
                            } else {
                                Value::Double(left.max(right))
                            }
                        }
                        (Value::Long(left), Value::Long(right)) => {
                            if method_name == "min" {
                                Value::Long(left.min(right))
                            } else {
                                Value::Long(left.max(right))
                            }
                        }
                        (Value::Int(left), Value::Int(right)) => {
                            if method_name == "min" {
                                Value::Int(left.min(right))
                            } else {
                                Value::Int(left.max(right))
                            }
                        }
                        _ => Value::Int(0),
                    })
                }
                _ => Err(self.error(
                    0,
                    0,
                    format!("java.lang.Math method {method_name} is not implemented"),
                )),
            };
        }
        if method_name == "<init>"
            && (class_name.starts_with("Lcom/badlogic/gdx/")
                || class_name.starts_with("Landroid/")
                || class_name.starts_with("Ljava/lang/ref/")
                || class_name.starts_with("Ljava/lang/")
                || class_name == "Ljava/lang/Enum;"
                || class_name == "Ljava/lang/Object;"
                || class_name == "Ljava/util/ArrayList;"
                || class_name == "Ljava/util/LinkedList;"
                || class_name == "Ljava/util/HashMap;"
                || class_name == "Ljava/util/Vector;"
                || class_name == "Ljava/lang/StringBuilder;")
        {
            if class_name == "Ljava/util/ArrayList;"
                || class_name == "Ljava/util/LinkedList;"
                || class_name == "Ljava/util/HashMap;"
                || class_name == "Ljava/util/Vector;"
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
        if class_name == "Ljava/lang/Math;" {
            return match method_name {
                "round" => Ok(Value::Int(match args.first() {
                    Some(Value::Float(value)) => value.round() as i32,
                    Some(Value::Double(value)) => value.round() as i32,
                    Some(Value::Int(value)) => *value,
                    Some(Value::Long(value)) => *value as i32,
                    _ => 0,
                })),
                "abs" => Ok(match args.first() {
                    Some(Value::Float(value)) => Value::Float(value.abs()),
                    Some(Value::Double(value)) => Value::Double(value.abs()),
                    Some(Value::Long(value)) => Value::Long(value.abs()),
                    Some(Value::Int(value)) => Value::Int(value.abs()),
                    _ => Value::Int(0),
                }),
                "sqrt" => Ok(Value::Double(
                    as_double(args.first().cloned().unwrap_or(Value::Int(0)), 0, 0)?.sqrt(),
                )),
                "sin" => Ok(Value::Double(
                    as_double(args.first().cloned().unwrap_or(Value::Int(0)), 0, 0)?.sin(),
                )),
                "cos" => Ok(Value::Double(
                    as_double(args.first().cloned().unwrap_or(Value::Int(0)), 0, 0)?.cos(),
                )),
                _ => Err(self.error(
                    0,
                    0,
                    format!("java.lang.Math method {method_name} is not implemented"),
                )),
            };
        }
        if class_name == "Ljava/lang/Object;" && method_name == "<init>" {
            return Ok(Value::Void);
        }
        if class_name == "Ljava/lang/Object;" && method_name == "getClass" {
            if let Some(Value::Object(id)) = args.first() {
                if let Some(HeapObject::Instance { class_name, .. }) = self.heap_object(*id) {
                    return Ok(Value::Object(
                        self.alloc(HeapObject::Class(class_name.clone())),
                    ));
                }
            }
            return Ok(Value::Null);
        }
        if class_name == "Ljava/lang/System;" {
            match method_name {
                "arraycopy" => {
                    let source = object_arg(args, 0)?;
                    let source_pos = int_arg(args, 1)? as usize;
                    let target = object_arg(args, 2)?;
                    let target_pos = int_arg(args, 3)? as usize;
                    let length = int_arg(args, 4)? as usize;
                    let values = match self.heap_object(source) {
                        Some(HeapObject::Array { values, .. }) => values.clone(),
                        _ => {
                            return Err(self.error(0, 0, "System.arraycopy source is not an array"))
                        }
                    };
                    if source_pos
                        .checked_add(length)
                        .is_none_or(|end| end > values.len())
                    {
                        return Err(self.error(0, 0, "System.arraycopy source range is invalid"));
                    }
                    let copied = values[source_pos..source_pos + length].to_vec();
                    match self.heap.get_mut(target as usize) {
                        Some(HeapObject::Array { values, .. }) => {
                            if target_pos
                                .checked_add(length)
                                .is_none_or(|end| end > values.len())
                            {
                                return Err(self.error(
                                    0,
                                    0,
                                    "System.arraycopy target range is invalid",
                                ));
                            }
                            values[target_pos..target_pos + length].clone_from_slice(&copied);
                        }
                        _ => {
                            return Err(self.error(0, 0, "System.arraycopy target is not an array"))
                        }
                    }
                    return Ok(Value::Void);
                }
                "currentTimeMillis" => return Ok(Value::Long(0)),
                "identityHashCode" => return Ok(Value::Int(0)),
                _ => {}
            }
        }
        if class_name.starts_with("Landroid/view/")
            || class_name.starts_with("Landroid/widget/")
            || class_name == "Landroid/opengl/GLSurfaceView;"
        {
            return match method_name {
                "getCurrentView" => {
                    let view =
                        self.alloc_instance("Lde/nurogames/android/tinysanta/views/ViewPlus;");
                    Ok(Value::Object(view))
                }
                "getChildAt" => {
                    let view =
                        self.alloc_instance("Lde/nurogames/android/tinysanta/views/ViewPlus;");
                    Ok(Value::Object(view))
                }
                "setDisplayedChild" | "startAnimation" | "setAnimation" => Ok(Value::Void),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Ljava/util/Locale;" {
            return match method_name {
                "getDefault" => Ok(Value::Object(self.alloc_instance("Ljava/util/Locale;"))),
                "getCountry" => Ok(Value::String("EN".to_owned())),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/os/Environment;" && method_name == "getExternalStorageDirectory"
        {
            return Ok(Value::Object(self.alloc_instance("Ljava/io/File;")));
        }
        if class_name == "Landroid/content/Context;" || class_name.starts_with("Landroid/app/") {
            return match method_name {
                "getApplicationContext" => object_arg(args, 0).map(Value::Object),
                "getResources" => Ok(Value::Object(
                    self.alloc_instance("Landroid/content/res/Resources;"),
                )),
                "getSystemService" => {
                    let name = self.string_arg(args, 1).unwrap_or_default();
                    match self.framework_call(FrameworkCall::GetSystemService { name })? {
                        FrameworkResult::Object(value) => Ok(Value::Object(value)),
                        _ => Ok(Value::Null),
                    }
                }
                "getWindowManager" => Ok(Value::Object(
                    self.alloc_instance("Landroid/view/WindowManager;"),
                )),
                "getWindow" => Ok(Value::Object(self.alloc_instance("Landroid/view/Window;"))),
                "getMainLooper" => Ok(Value::Object(self.alloc_instance("Landroid/os/Looper;"))),
                "registerReceiver"
                | "unregisterReceiver"
                | "setContentView"
                | "setRequestedOrientation"
                | "requestWindowFeature" => Ok(Value::Void),
                "findViewById" => Ok(Value::Object(self.alloc_instance("Landroid/view/View;"))),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/view/Window;" {
            return match method_name {
                "getWindowManager" => Ok(Value::Object(
                    self.alloc_instance("Landroid/view/WindowManager;"),
                )),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/view/Window;" || class_name.starts_with("Landroid/app/") {
            return Ok(Value::Void);
        }
        if class_name == "Landroid/view/Window;" || class_name.starts_with("Landroid/app/") {
            return Ok(Value::Void);
        }
        if class_name == "Landroid/hardware/SensorManager;" {
            return match method_name {
                "getDefaultSensor" => Ok(Value::Object(
                    self.alloc_instance("Landroid/hardware/Sensor;"),
                )),
                "getSensorList" => Ok(Value::Object(
                    self.alloc(HeapObject::Collection(Vec::new())),
                )),
                "registerListener" => Ok(Value::Int(1)),
                "unregisterListener" => Ok(Value::Void),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/view/WindowManager;" {
            return match method_name {
                "getDefaultDisplay" => {
                    Ok(Value::Object(self.alloc_instance("Landroid/view/Display;")))
                }
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/view/Display;" {
            return match method_name {
                "getWidth" | "getHeight" => Ok(Value::Int(320)),
                "getMetrics" => Ok(Value::Void),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Landroid/media/MediaPlayer;" {
            return match method_name {
                "create" => Ok(Value::Object(
                    self.alloc_instance("Landroid/media/MediaPlayer;"),
                )),
                "isPlaying" => Ok(Value::Int(0)),
                "start"
                | "pause"
                | "stop"
                | "reset"
                | "release"
                | "prepare"
                | "prepareAsync"
                | "setLooping"
                | "setVolume"
                | "seekTo"
                | "setOnCompletionListener"
                | "setOnPreparedListener" => Ok(Value::Void),
                _ => Ok(Value::Void),
            };
        }
        if class_name == "Ljava/lang/reflect/Array;" && method_name == "newInstance" {
            let component = match args.first() {
                Some(Value::Object(id)) => match self.heap_object(*id) {
                    Some(HeapObject::Class(name)) => name.clone(),
                    _ => "Ljava/lang/Object;".to_owned(),
                },
                _ => "Ljava/lang/Object;".to_owned(),
            };
            let dimensions = match args.get(1) {
                Some(Value::Object(id)) => match self.heap_object(*id) {
                    Some(HeapObject::Array { values, .. }) => values
                        .iter()
                        .map(|value| match value {
                            Value::Int(value) => (*value).max(0) as usize,
                            Value::Long(value) => (*value).max(0) as usize,
                            _ => 0,
                        })
                        .collect::<Vec<_>>(),
                    _ => vec![0],
                },
                _ => vec![int_arg(args, 1)?.max(0) as usize],
            };
            return Ok(Value::Object(
                self.alloc_reflective_array(&component, &dimensions),
            ));
        }
        if class_name == "Ljava/io/File;" {
            let receiver = object_arg(args, 0)?;
            return Ok(match method_name {
                "exists" => Value::Int(0),
                "delete" => Value::Int(1),
                "toString" => Value::Object(receiver),
                _ => Value::Void,
            });
        }
        if class_name == "Ljava/lang/Integer;"
            || class_name == "Ljava/lang/Long;"
            || class_name == "Ljava/lang/Float;"
            || class_name == "Ljava/lang/Double;"
            || class_name == "Ljava/lang/Boolean;"
        {
            match method_name {
                "valueOf" => {
                    let value = args.first().cloned().unwrap_or(Value::Int(0));
                    return Ok(Value::Object(self.alloc(HeapObject::Boxed(value))));
                }
                "intValue" => return Ok(Value::Int(int_arg(args, 0)?)),
                "longValue" => return Ok(Value::Long(int_arg(args, 0)? as i64)),
                "floatValue" => return Ok(Value::Float(float_arg(args, 0)?)),
                "doubleValue" => return Ok(Value::Double(float_arg(args, 0)? as f64)),
                "booleanValue" => return Ok(Value::Int(int_arg(args, 0)?)),
                "toString" => {
                    return Ok(Value::Object(self.alloc_string(format!(
                        "{:?}",
                        args.first().unwrap_or(&Value::Int(0))
                    ))))
                }
                _ => {}
            }
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
                "equalsIgnoreCase" => {
                    let left = value_of(args.first()).to_ascii_lowercase();
                    let right = value_of(args.get(1)).to_ascii_lowercase();
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
                "split" => {
                    let value = value_of(args.first());
                    let separator = value_of(args.get(1));
                    let values = if separator.is_empty() {
                        value
                            .chars()
                            .map(|c| Value::Object(self.alloc_string(c.to_string())))
                            .collect()
                    } else {
                        value
                            .split(&separator)
                            .map(|part| Value::Object(self.alloc_string(part.to_owned())))
                            .collect()
                    };
                    let array = self.alloc(HeapObject::Array {
                        component: "Ljava/lang/String;".to_owned(),
                        values,
                    });
                    return Ok(Value::Object(array));
                }
                "charAt" => {
                    let value = value_of(args.first());
                    let index = int_arg(args, 1)? as usize;
                    let ch = value.chars().nth(index).unwrap_or('\0') as i32;
                    return Ok(Value::Int(ch));
                }
                "indexOf" => {
                    let value = value_of(args.first());
                    let needle = value_of(args.get(1));
                    return Ok(Value::Int(
                        value.find(&needle).map_or(-1, |index| index as i32),
                    ));
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
            || class_name == "Ljava/util/Vector;"
            || class_name == "Ljava/util/Collection;"
            || class_name == "Ljava/util/List;"
            || class_name == "Ljava/util/AbstractList;"
            || class_name == "Ljava/util/AbstractCollection;"
        {
            let receiver = match args.first() {
                Some(Value::Object(receiver)) => *receiver,
                _ => self.alloc(HeapObject::Collection(Vec::new())),
            };
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
                "add" | "addElement" => {
                    if let Some(HeapObject::Collection(values)) =
                        self.heap.get_mut(receiver as usize)
                    {
                        values.push(args.get(1).cloned().unwrap_or(Value::Null));
                    }
                    return Ok(if method_name == "add" {
                        Value::Int(1)
                    } else {
                        Value::Void
                    });
                }
                "get" | "elementAt" => {
                    let index = int_arg(args, 1)? as usize;
                    return Ok(match self.heap_object(receiver) {
                        Some(HeapObject::Collection(values)) => {
                            values.get(index).cloned().unwrap_or(Value::Null)
                        }
                        _ => Value::Null,
                    });
                }
                "clear" => {
                    if let Some(HeapObject::Collection(values)) =
                        self.heap.get_mut(receiver as usize)
                    {
                        values.clear();
                    }
                    return Ok(Value::Void);
                }
                _ => return Ok(Value::Void),
            }
        }
        if class_name == "Ljava/lang/Class;" && method_name == "getMethod" {
            return Ok(Value::Object(
                self.alloc_instance("Ljava/lang/reflect/Method;"),
            ));
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
        Err(self.error(
            0,
            0,
            format!("framework method {class_name}->{method_name} is not implemented"),
        ))
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
                self.framework.gdx_listener = args.get(1).and_then(|value| match value {
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
            ("Lcom/badlogic/gdx/Graphics;", "getDeltaTime")
            | ("Lcom/badlogic/gdx/Graphics;", "getRawDeltaTime") => {
                FrameworkResult::Float(1.0 / 60.0)
            }
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
                let path = args
                    .get(1)
                    .map(|value| self.string_arg(std::slice::from_ref(value), 0))
                    .transpose()?
                    .unwrap_or_default();
                let handle = self.alloc_instance("Lcom/badlogic/gdx/files/FileHandle;");
                if let Some(HeapObject::Instance { fields, .. }) =
                    self.heap.get_mut(handle as usize)
                {
                    fields.insert("path".to_owned(), Value::String(path));
                }
                FrameworkResult::Object(handle)
            }
            ("Lcom/badlogic/gdx/files/FileHandle;", "exists")
            | ("Lcom/badlogic/gdx/files/FileHandle;", "length")
            | ("Lcom/badlogic/gdx/files/FileHandle;", "readBytes")
            | ("Lcom/badlogic/gdx/files/FileHandle;", "readString") => {
                let handle = args.first().and_then(|value| match value {
                    Value::Object(id) => Some(*id),
                    _ => None,
                });
                let path =
                    handle
                        .and_then(|id| self.heap_object(id))
                        .and_then(|value| match value {
                            HeapObject::Instance { fields, .. } => fields.get("path"),
                            _ => None,
                        });
                let result = self.framework.assets.as_ref().and_then(|assets| {
                    path.and_then(|value| match value {
                        Value::String(path) => assets.read(path),
                        _ => None,
                    })
                });
                match method_name {
                    "exists" => FrameworkResult::Bool(result.is_some()),
                    "length" => {
                        FrameworkResult::Long(result.as_ref().map_or(0, |bytes| bytes.len() as i64))
                    }
                    "readBytes" => result.map_or(FrameworkResult::Object(0), |bytes| {
                        FrameworkResult::Object(
                            self.alloc(HeapObject::Array {
                                component: "B".to_owned(),
                                values: bytes
                                    .into_iter()
                                    .map(|byte| Value::Int(i32::from(byte)))
                                    .collect(),
                            }),
                        )
                    }),
                    _ => FrameworkResult::String(result.map_or_else(String::new, |bytes| {
                        String::from_utf8_lossy(&bytes).into_owned()
                    })),
                }
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
            ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "<init>") => {
                let receiver = object_arg(args, 0)?;
                if let Some(path) = args
                    .get(1)
                    .map(|value| self.string_arg(std::slice::from_ref(value), 0))
                    .transpose()?
                {
                    self.set_object_field(receiver, "path", Value::String(path));
                }
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/Texture;", "<init>") => {
                let receiver = object_arg(args, 0)?;
                if let Some(path) = args
                    .get(1)
                    .map(|value| self.string_arg(std::slice::from_ref(value), 0))
                    .transpose()?
                {
                    self.set_object_field(receiver, "path", Value::String(path));
                }
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureRegion;", "<init>") => {
                let receiver = object_arg(args, 0)?;
                if let Some(texture) = args.get(1).and_then(object_id) {
                    self.copy_drawable_fields(texture, receiver);
                    if self.object_field_float(receiver, "region_width").is_none() {
                        if let Some(path) = self
                            .object_field_string(receiver, "path")
                            .or_else(|| self.object_field_string(receiver, "asset_path"))
                        {
                            if let Some((width, height)) = self
                                .framework
                                .assets
                                .as_ref()
                                .and_then(|assets| assets.image_size(&path))
                            {
                                self.set_object_field(
                                    receiver,
                                    "region_width",
                                    Value::Int(width as i32),
                                );
                                self.set_object_field(
                                    receiver,
                                    "region_height",
                                    Value::Int(height as i32),
                                );
                            }
                        }
                    }
                }
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "findRegion") => {
                let region =
                    self.alloc_instance("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;");
                if let (Some(Value::Object(atlas)), Some(name)) = (
                    args.first(),
                    args.get(1).and_then(|value| match value {
                        Value::String(name) => Some(name.clone()),
                        Value::Object(id) => match self.heap_object(*id) {
                            Some(HeapObject::String(name)) => Some(name.clone()),
                            _ => None,
                        },
                        _ => None,
                    }),
                ) {
                    let atlas_path = self.object_field_string(*atlas, "path").unwrap_or_default();
                    if let Some(asset) = self
                        .framework
                        .assets
                        .as_ref()
                        .and_then(|assets| assets.atlas_region(&atlas_path, &name))
                    {
                        self.set_object_field(region, "asset_path", Value::String(asset.page));
                        self.set_object_field(region, "region_x", Value::Int(asset.x as i32));
                        self.set_object_field(region, "region_y", Value::Int(asset.y as i32));
                        self.set_object_field(
                            region,
                            "region_width",
                            Value::Int(asset.width as i32),
                        );
                        self.set_object_field(
                            region,
                            "region_height",
                            Value::Int(asset.height as i32),
                        );
                    }
                }
                FrameworkResult::Object(region)
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "findRegions") => {
                FrameworkResult::Object(self.alloc_collection())
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "createSprite")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas;", "newSprite") => {
                let sprite = self.alloc_instance("Lcom/badlogic/gdx/graphics/g2d/Sprite;");
                if let Some(region) = args.get(1).and_then(object_id) {
                    self.copy_drawable_fields(region, sprite);
                    if let Some(width) = self.object_field_float(region, "region_width") {
                        self.set_object_field(sprite, "width", Value::Float(width));
                    }
                    if let Some(height) = self.object_field_float(region, "region_height") {
                        self.set_object_field(sprite, "height", Value::Float(height));
                    }
                }
                FrameworkResult::Object(sprite)
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
                let receiver = object_arg(args, 0)?;
                let field = match method_name {
                    "getRegionWidth" => "region_width",
                    "getRegionHeight" => "region_height",
                    "getRegionX" => "region_x",
                    _ => "region_y",
                };
                FrameworkResult::Int(self.object_field_float(receiver, field).unwrap_or(0.0) as i32)
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureRegion;", "getTexture")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;", "getTexture") => {
                let receiver = object_arg(args, 0)?;
                let texture = self.alloc_instance("Lcom/badlogic/gdx/graphics/Texture;");
                self.copy_drawable_fields(receiver, texture);
                FrameworkResult::Object(texture)
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;", "<init>") => {
                let region = object_arg(args, 0)?;
                if let Some(drawable) = args.get(1).and_then(object_id) {
                    self.copy_drawable_fields(drawable, region);
                    if self.object_field_float(region, "region_width").is_none() {
                        if let Some(path) = self.object_field_string(region, "path") {
                            if let Some((width, height)) = self
                                .framework
                                .assets
                                .as_ref()
                                .and_then(|assets| assets.image_size(&path))
                            {
                                self.set_object_field(region, "region_x", Value::Int(0));
                                self.set_object_field(region, "region_y", Value::Int(0));
                                self.set_object_field(
                                    region,
                                    "region_width",
                                    Value::Int(width as i32),
                                );
                                self.set_object_field(
                                    region,
                                    "region_height",
                                    Value::Int(height as i32),
                                );
                            }
                        }
                    }
                }
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/TextureRegion;", "setRegion")
            | ("Lcom/badlogic/gdx/graphics/g2d/TextureAtlas$AtlasRegion;", "setRegion") => {
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "<init>") => {
                let sprite = object_arg(args, 0)?;
                if let Some(drawable) = args.get(1).and_then(object_id) {
                    for field in [
                        "asset_path",
                        "region_x",
                        "region_y",
                        "region_width",
                        "region_height",
                    ] {
                        if let Some(value) =
                            self.heap_object(drawable).and_then(|object| match object {
                                HeapObject::Instance { fields, .. } => fields.get(field).cloned(),
                                _ => None,
                            })
                        {
                            self.set_object_field(sprite, field, value);
                        }
                    }
                    if let Some(width) = self.object_field_float(drawable, "region_width") {
                        self.set_object_field(sprite, "width", Value::Float(width));
                    }
                    if let Some(height) = self.object_field_float(drawable, "region_height") {
                        self.set_object_field(sprite, "height", Value::Float(height));
                    }
                }
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setOrigin")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setRotation")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "translate")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "rotate") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setRegion")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setTexture") => {
                let sprite = object_arg(args, 0)?;
                if let Some(drawable) = args.get(1).and_then(object_id) {
                    self.copy_drawable_fields(drawable, sprite);
                    if let Some(width) = self.object_field_float(drawable, "region_width") {
                        self.set_object_field(sprite, "width", Value::Float(width));
                    }
                    if let Some(height) = self.object_field_float(drawable, "region_height") {
                        self.set_object_field(sprite, "height", Value::Float(height));
                    }
                }
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setColor") => {
                let sprite = object_arg(args, 0)?;
                let color = color_args(self, args, 1)?;
                self.set_object_field(sprite, "color", Value::Int(pack_color(color)));
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setPosition") => {
                let sprite = object_arg(args, 0)?;
                self.set_object_field(sprite, "x", Value::Float(float_arg(args, 1)?));
                self.set_object_field(sprite, "y", Value::Float(float_arg(args, 2)?));
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setScale") => {
                let sprite = object_arg(args, 0)?;
                let scale_x = float_arg(args, 1)?;
                let scale_y = args
                    .get(2)
                    .map(|_| float_arg(args, 2))
                    .transpose()?
                    .unwrap_or(scale_x);
                self.set_object_field(sprite, "scale_x", Value::Float(scale_x));
                self.set_object_field(sprite, "scale_y", Value::Float(scale_y));
                if let Some(region_width) = self.object_field_float(sprite, "region_width") {
                    self.set_object_field(
                        sprite,
                        "width",
                        Value::Float(region_width * scale_x.abs()),
                    );
                }
                if let Some(region_height) = self.object_field_float(sprite, "region_height") {
                    self.set_object_field(
                        sprite,
                        "height",
                        Value::Float(region_height * scale_y.abs()),
                    );
                }
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setSize") => {
                let sprite = object_arg(args, 0)?;
                self.set_object_field(sprite, "width", Value::Float(float_arg(args, 1)?));
                self.set_object_field(sprite, "height", Value::Float(float_arg(args, 2)?));
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "setBounds") => {
                let sprite = object_arg(args, 0)?;
                self.set_object_field(sprite, "x", Value::Float(float_arg(args, 1)?));
                self.set_object_field(sprite, "y", Value::Float(float_arg(args, 2)?));
                self.set_object_field(sprite, "width", Value::Float(float_arg(args, 3)?));
                self.set_object_field(sprite, "height", Value::Float(float_arg(args, 4)?));
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getX")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getY")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getWidth")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getHeight")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getRotation")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getScaleX")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getScaleY") => {
                let sprite = object_arg(args, 0)?;
                let field = match method_name {
                    "getX" => "x",
                    "getY" => "y",
                    "getWidth" => "width",
                    "getHeight" => "height",
                    "getScaleX" => "scale_x",
                    "getScaleY" => "scale_y",
                    _ => "rotation",
                };
                let fallback = match method_name {
                    "getScaleX" | "getScaleY" => 1.0,
                    _ => 0.0,
                };
                FrameworkResult::Float(self.object_field_float(sprite, field).unwrap_or(fallback))
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "getColor") => {
                FrameworkResult::Object(self.alloc_instance("Lcom/badlogic/gdx/graphics/Color;"))
            }
            ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "getColor") => {
                FrameworkResult::Object(self.alloc_instance("Lcom/badlogic/gdx/graphics/Color;"))
            }
            ("Lcom/badlogic/gdx/graphics/Texture;", "dispose")
            | ("Lcom/badlogic/gdx/graphics/Texture;", "setFilter")
            | ("Lcom/badlogic/gdx/graphics/Texture;", "setWrap") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/graphics/Texture;", "getWidth")
            | ("Lcom/badlogic/gdx/graphics/Texture;", "getHeight") => {
                let handle = args.first().and_then(|value| match value {
                    Value::Object(id) => Some(*id),
                    _ => None,
                });
                let path = handle.and_then(|id| self.object_field_string(id, "path"));
                let size = self
                    .framework
                    .assets
                    .as_ref()
                    .and_then(|assets| path.and_then(|path| assets.image_size(&path)))
                    .ok_or_else(|| self.error(0, 0, "Texture dimensions unavailable: asset was not found or could not be decoded"))?;
                FrameworkResult::Int(if method_name == "getWidth" {
                    size.0 as i32
                } else {
                    size.1 as i32
                })
            }
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
                    2 if matches!(args.first(), Some(Value::Float(_) | Value::Double(_)))
                        || matches!(args.get(1), Some(Value::Float(_) | Value::Double(_))) =>
                    {
                        let low = float_arg(args, 0)?;
                        let high = float_arg(args, 1)?;
                        low + (high - low) * ((self.executed_steps % 1000) as f32 / 1000.0)
                    }
                    2 => {
                        let low = int_arg(args, 0)?;
                        let high = int_arg(args, 1)?;
                        let (low, high) = if high < low { (high, low) } else { (low, high) };
                        let span = i64::from(high) - i64::from(low) + 1;
                        (i64::from(low) + self.executed_steps as i64 % span) as f32
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
            | ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "setBlendFunction")
            | ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "flush") => FrameworkResult::Void,
            ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "setColor") => {
                let batch = object_arg(args, 0)?;
                let color = color_args(self, args, 1)?;
                self.set_object_field(batch, "color", Value::Int(pack_color(color)));
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "draw") => {
                let sprite = object_arg(args, 0)?;
                let (x, y, width, height) = self.draw_bounds(&[Value::Object(sprite)]);
                let texture_path = self
                    .object_field_string(sprite, "path")
                    .or_else(|| self.object_field_string(sprite, "asset_path"));
                let region = ["region_x", "region_y", "region_width", "region_height"]
                    .iter()
                    .map(|field| self.object_field_float(sprite, field))
                    .collect::<Option<Vec<_>>>()
                    .map(|values| (values[0], values[1], values[2], values[3]));
                let color = self
                    .object_field_int(sprite, "color")
                    .map(unpack_color)
                    .unwrap_or(Rgba8 {
                        r: 255,
                        g: 255,
                        b: 255,
                        a: 255,
                    });
                if let Some(path) = texture_path {
                    let _ = self.render_asset(&path, x, y, width, height, region, color);
                }
                FrameworkResult::Void
            }
            ("Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;", "draw")
            | ("Lcom/badlogic/gdx/graphics/g2d/Sprite;", "render")
            | ("Lcom/badlogic/gdx/graphics/g2d/BitmapFont;", "draw") => {
                if class_name == "Lcom/badlogic/gdx/graphics/g2d/SpriteBatch;" && args.len() >= 10 {
                    let drawable = object_arg(args, 1)?;
                    let path = self
                        .object_field_string(drawable, "path")
                        .or_else(|| self.object_field_string(drawable, "asset_path"));
                    let x = float_arg(args, 2)?;
                    let y = float_arg(args, 3)?;
                    let width = float_arg(args, 6)?;
                    let height = float_arg(args, 7)?;
                    let region = ["region_x", "region_y", "region_width", "region_height"]
                        .iter()
                        .map(|field| self.object_field_float(drawable, field))
                        .collect::<Option<Vec<_>>>()
                        .map(|values| (values[0], values[1], values[2], values[3]));
                    let color = self
                        .object_field_int(args.first().and_then(object_id).unwrap_or(0), "color")
                        .map(unpack_color)
                        .unwrap_or(Rgba8 {
                            r: 255,
                            g: 255,
                            b: 255,
                            a: 255,
                        });
                    if let Some(path) = path {
                        let _ = self.render_asset(&path, x, y, width, height, region, color);
                    }
                    return Ok(Value::Void);
                }
                let (x, y, width, height) = self.draw_bounds(args);
                let drawable = args.get(1).and_then(object_id).or_else(|| {
                    (class_name == "Lcom/badlogic/gdx/graphics/g2d/Sprite;")
                        .then(|| object_arg(args, 0).ok())
                        .flatten()
                });
                let texture_path = drawable.and_then(|id| {
                    self.object_field_string(id, "path")
                        .or_else(|| self.object_field_string(id, "asset_path"))
                });
                let region = drawable.and_then(|id| {
                    let x = self.object_field_float(id, "region_x")?;
                    let y = self.object_field_float(id, "region_y")?;
                    let width = self.object_field_float(id, "region_width")?;
                    let height = self.object_field_float(id, "region_height")?;
                    Some((x, y, width, height))
                });
                let color = drawable
                    .and_then(|id| self.object_field_int(id, "color"))
                    .map(unpack_color)
                    .unwrap_or(Rgba8 {
                        r: 255,
                        g: 255,
                        b: 255,
                        a: 255,
                    });
                let rendered = texture_path.and_then(|path| {
                    self.render_asset(&path, x, y, width, height, region, color)
                        .then_some(())
                });
                if rendered.is_none() {
                    self.framework.gles.draw_quad_pixels(
                        x,
                        y,
                        width,
                        height,
                        Rgba8 {
                            r: 220,
                            g: 235,
                            b: 240,
                            a: 255,
                        },
                    );
                }
                FrameworkResult::Void
            }
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
            FrameworkResult::Float(value) => Value::Float(value),
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
                Some(HeapObject::Instance { fields, .. }) => match fields.get("path") {
                    Some(Value::String(value)) => Ok(value.clone()),
                    Some(Value::Object(path)) => match self.heap_object(*path) {
                        Some(HeapObject::String(value)) => Ok(value.clone()),
                        _ => Err(self.error(
                            0,
                            0,
                            format!(
                                "framework argument {index} is object {id}, expected a path string"
                            ),
                        )),
                    },
                    _ => Err(self.error(
                        0,
                        0,
                        format!(
                            "framework argument {index} is object {id}, expected a path string"
                        ),
                    )),
                },
                _ => Err(self.error(
                    0,
                    0,
                    format!("framework argument {index} is object {id}, expected java.lang.String"),
                )),
            },
            _ => Err(self.error(0, 0, format!("framework argument {index} is not a string"))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_asset(
        &mut self,
        path: &str,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        region: Option<(f32, f32, f32, f32)>,
        color: Rgba8,
    ) -> bool {
        let Some(image) = self
            .framework
            .assets
            .as_ref()
            .and_then(|assets| assets.image(path).ok())
        else {
            return false;
        };
        let texture = self
            .framework
            .gles
            .upload_texture(image.width, image.height, &image.pixels);
        if let Some((region_x, region_y, region_width, region_height)) = region {
            self.framework.gles.draw_textured_region_pixels(
                x,
                y,
                width,
                height,
                texture,
                region_x / image.width.max(1) as f32,
                region_y / image.height.max(1) as f32,
                (region_x + region_width) / image.width.max(1) as f32,
                (region_y + region_height) / image.height.max(1) as f32,
                color,
            );
        } else {
            self.framework
                .gles
                .draw_textured_quad_pixels(x, y, width, height, texture, color);
        }
        true
    }

    fn object_field_int(&self, object: ObjectId, name: &str) -> Option<i32> {
        match self.heap_object(object).and_then(|value| match value {
            HeapObject::Instance { fields, .. } => fields.get(name),
            _ => None,
        }) {
            Some(Value::Int(value)) => Some(*value),
            Some(Value::Long(value)) => Some(*value as i32),
            _ => None,
        }
    }

    fn object_field_string(&self, object: ObjectId, name: &str) -> Option<String> {
        match self.heap_object(object) {
            Some(HeapObject::Instance { fields, .. }) => match fields.get(name) {
                Some(Value::String(value)) => Some(value.clone()),
                Some(Value::Object(id)) => match self.heap_object(*id) {
                    Some(HeapObject::String(value)) => Some(value.clone()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    fn set_object_field(&mut self, object: ObjectId, name: &str, value: Value) {
        if let Some(HeapObject::Instance { fields, .. }) = self.heap.get_mut(object as usize) {
            fields.insert(name.to_owned(), value);
        }
    }

    fn copy_drawable_fields(&mut self, source: ObjectId, destination: ObjectId) {
        for field in [
            "asset_path",
            "path",
            "region_x",
            "region_y",
            "region_width",
            "region_height",
            "width",
            "height",
        ] {
            if let Some(value) = self.heap_object(source).and_then(|object| match object {
                HeapObject::Instance { fields, .. } => fields.get(field).cloned(),
                _ => None,
            }) {
                self.set_object_field(destination, field, value);
            }
        }
    }

    fn draw_bounds(&self, args: &[Value]) -> (f32, f32, f32, f32) {
        let receiver = args.first().and_then(object_id);
        let drawable = args.get(1).and_then(object_id).or_else(|| {
            let receiver = receiver?;
            self.heap_object(receiver)
                .is_some_and(|object| matches!(object, HeapObject::Instance { class_name, .. } if class_name == "Lcom/badlogic/gdx/graphics/g2d/Sprite;"))
                .then_some(receiver)
        });
        let x = receiver
            .and_then(|id| self.object_field_float(id, "x"))
            .or_else(|| numeric_value(args.get(2)))
            .unwrap_or(0.0);
        let y = receiver
            .and_then(|id| self.object_field_float(id, "y"))
            .or_else(|| numeric_value(args.get(3)))
            .unwrap_or(0.0);
        let drawable_width = drawable.and_then(|id| {
            self.object_field_float(id, "region_width")
                .filter(|value| value.is_finite() && *value > 0.0)
                .or_else(|| {
                    let path = self
                        .object_field_string(id, "path")
                        .or_else(|| self.object_field_string(id, "asset_path"))?;
                    self.framework
                        .assets
                        .as_ref()
                        .and_then(|assets| assets.image_size(&path))
                        .map(|(width, _)| width as f32)
                })
        });
        let drawable_height = drawable.and_then(|id| {
            self.object_field_float(id, "region_height")
                .filter(|value| value.is_finite() && *value > 0.0)
                .or_else(|| {
                    let path = self
                        .object_field_string(id, "path")
                        .or_else(|| self.object_field_string(id, "asset_path"))?;
                    self.framework
                        .assets
                        .as_ref()
                        .and_then(|assets| assets.image_size(&path))
                        .map(|(_, height)| height as f32)
                })
        });

        let width = receiver
            .and_then(|id| {
                self.object_field_float(id, "width")
                    .filter(|value| value.is_finite() && *value > 0.0)
            })
            .or_else(|| {
                receiver.and_then(|id| {
                    self.object_field_float(id, "region_width")
                        .filter(|value| value.is_finite() && *value > 0.0)
                })
            })
            .or_else(|| numeric_value(args.get(4)).filter(|value| *value > 0.0))
            .or(drawable_width)
            .unwrap_or(1.0);
        let height = receiver
            .and_then(|id| {
                self.object_field_float(id, "height")
                    .filter(|value| value.is_finite() && *value > 0.0)
            })
            .or_else(|| {
                receiver.and_then(|id| {
                    self.object_field_float(id, "region_height")
                        .filter(|value| value.is_finite() && *value > 0.0)
                })
            })
            .or_else(|| numeric_value(args.get(5)).filter(|value| *value > 0.0))
            .or(drawable_height)
            .unwrap_or(1.0);
        (x, y, width.max(1.0), height.max(1.0))
    }

    fn object_field_float(&self, object: ObjectId, name: &str) -> Option<f32> {
        match self.heap_object(object).and_then(|value| match value {
            HeapObject::Instance { fields, .. } => fields.get(name),
            _ => None,
        }) {
            Some(Value::Float(value)) => Some(*value),
            Some(Value::Double(value)) => Some(*value as f32),
            Some(Value::Int(value)) => Some(*value as f32),
            _ => None,
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

fn color_args(vm: &Vm<'_>, args: &[Value], index: usize) -> Result<Rgba8, VmError> {
    if args.len() >= index + 4 {
        return Ok(Rgba8 {
            r: (float_arg(args, index)? * 255.0).clamp(0.0, 255.0) as u8,
            g: (float_arg(args, index + 1)? * 255.0).clamp(0.0, 255.0) as u8,
            b: (float_arg(args, index + 2)? * 255.0).clamp(0.0, 255.0) as u8,
            a: (float_arg(args, index + 3)? * 255.0).clamp(0.0, 255.0) as u8,
        });
    }
    let object = object_arg(args, index)?;
    let red = vm.object_field_float(object, "r").unwrap_or(1.0);
    let green = vm.object_field_float(object, "g").unwrap_or(1.0);
    let blue = vm.object_field_float(object, "b").unwrap_or(1.0);
    let alpha = vm.object_field_float(object, "a").unwrap_or(1.0);
    Ok(Rgba8 {
        r: (red * 255.0).clamp(0.0, 255.0) as u8,
        g: (green * 255.0).clamp(0.0, 255.0) as u8,
        b: (blue * 255.0).clamp(0.0, 255.0) as u8,
        a: (alpha * 255.0).clamp(0.0, 255.0) as u8,
    })
}

fn pack_color(color: Rgba8) -> i32 {
    i32::from_be_bytes([color.r, color.g, color.b, color.a])
}

fn unpack_color(value: i32) -> Rgba8 {
    let [r, g, b, a] = value.to_be_bytes();
    Rgba8 { r, g, b, a }
}

fn object_id(value: &Value) -> Option<ObjectId> {
    match value {
        Value::Object(id) => Some(*id),
        _ => None,
    }
}

fn numeric_value(value: Option<&Value>) -> Option<f32> {
    match value {
        Some(Value::Float(value)) => Some(*value),
        Some(Value::Double(value)) => Some(*value as f32),
        Some(Value::Int(value)) => Some(*value as f32),
        Some(Value::Long(value)) => Some(*value as f32),
        _ => None,
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

fn read_wide_register(
    registers: &[Value],
    index: usize,
    pc: usize,
    opcode: u8,
) -> Result<Value, VmError> {
    let value = registers.get(index).cloned().ok_or_else(|| VmError {
        pc,
        opcode,
        message: format!("wide register v{index} is outside the frame"),
    })?;
    if index + 1 >= registers.len() {
        return Err(VmError {
            pc,
            opcode,
            message: format!("wide register v{index} is outside the frame"),
        });
    }
    Ok(value)
}

fn set_wide_register(
    registers: &mut [Value],
    index: usize,
    value: Value,
    vm: &Vm<'_>,
    pc: usize,
    opcode: u8,
) -> Result<(), VmError> {
    if index + 1 >= registers.len() {
        return Err(vm.error(
            pc,
            opcode,
            format!("wide register v{index} is outside the frame"),
        ));
    }
    registers[index] = value;
    registers[index + 1] = Value::Void;
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

fn compare_float(left: f32, right: f32, nan_is_greater: bool) -> i32 {
    if left.is_nan() || right.is_nan() {
        return if nan_is_greater { 1 } else { -1 };
    }
    left.partial_cmp(&right)
        .map_or(0, |ordering| ordering as i32)
}

fn compare_long(left: i64, right: i64) -> i32 {
    match left.cmp(&right) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn compare_double(left: f64, right: f64, nan_is_greater: bool) -> i32 {
    if left.is_nan() || right.is_nan() {
        return if nan_is_greater { 1 } else { -1 };
    }
    left.partial_cmp(&right)
        .map_or(0, |ordering| ordering as i32)
}

fn default_value_for_type(type_name: &str) -> Value {
    match type_name {
        "I" | "Z" | "B" | "S" | "C" => Value::Int(0),
        "F" => Value::Float(0.0),
        "J" => Value::Long(0),
        "D" => Value::Double(0.0),
        _ => Value::Null,
    }
}

fn as_int(value: Value, pc: usize, opcode: u8) -> Result<i32, VmError> {
    match value {
        Value::Int(value) => Ok(value),
        Value::Null | Value::Void => Ok(0),
        _ => Err(VmError {
            pc,
            opcode,
            message: format!("value is not an integer: {value:?}"),
        }),
    }
}

fn as_float(value: Value, pc: usize, opcode: u8) -> Result<f32, VmError> {
    match value {
        Value::Float(value) => Ok(value),
        Value::Double(value) => Ok(value as f32),
        Value::Int(value) => Ok(value as f32),
        Value::Long(value) => Ok(value as f32),
        Value::Void | Value::Null => Ok(0.0),
        Value::Object(_) | Value::String(_) => Err(VmError {
            pc,
            opcode,
            message: "value is not a float".to_owned(),
        }),
    }
}

fn as_long(value: Value, pc: usize, opcode: u8) -> Result<i64, VmError> {
    match value {
        Value::Long(value) => Ok(value),
        Value::Int(value) => Ok(value as i64),
        Value::Null => Ok(0),
        _ => Err(VmError {
            pc,
            opcode,
            message: "value is not a long".to_owned(),
        }),
    }
}

fn as_double(value: Value, pc: usize, opcode: u8) -> Result<f64, VmError> {
    match value {
        Value::Double(value) => Ok(value),
        Value::Float(value) => Ok(value as f64),
        Value::Int(value) => Ok(value as f64),
        Value::Long(value) => Ok(value as f64),
        Value::Void | Value::Null => Ok(0.0),
        Value::Object(_) | Value::String(_) => Err(VmError {
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
        Some(Value::Void) | Some(Value::Null) => Ok(0.0),
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
        Some(Value::Void) | Some(Value::Null) => Ok(0),
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

fn values_equal(left: &Value, right: &Value) -> bool {
    left == right
}

fn compare_values(left: &Value, right: &Value, pc: usize, opcode: u8) -> Result<i32, VmError> {
    match (left, right) {
        (Value::Int(left), Value::Int(right)) => Ok(left.cmp(right) as i32),
        (Value::Long(left), Value::Long(right)) => Ok(left.cmp(right) as i32),
        (Value::Float(left), Value::Float(right)) => {
            Ok(left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal) as i32)
        }
        (Value::Double(left), Value::Double(right)) => {
            Ok(left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal) as i32)
        }
        (Value::Void, Value::Void) | (Value::Null, Value::Null) => Ok(0),
        (Value::Void, Value::Null) | (Value::Null, Value::Void) => Ok(0),
        _ => Err(VmError {
            pc,
            opcode,
            message: format!("values are not comparable: {left:?} and {right:?}"),
        }),
    }
}

fn branch_target(
    pc: usize,
    offset: i32,
    len: usize,
    at: usize,
    opcode: u8,
) -> Result<usize, VmError> {
    let target = pc as i64 + offset as i64;
    if target < 0 || target > len as i64 {
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
