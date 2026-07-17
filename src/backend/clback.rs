use std::collections::HashMap;

use cranelift::codegen::Context;
use cranelift::{codegen::ir::StackSlot, prelude::*};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::ir::ir::StructLayout;
use crate::ir::tac::{CastType, Instruction, IrOp, ScopedMap, Value};
use crate::parse::parsing::Type;
use crate::semantics::analysis::FunctionSignature;

fn strip_mangling(name: &str) -> &str {
    name.split("__")
        .next()
        .unwrap_or(name)
        .split('<')
        .next()
        .unwrap_or(name)
}

/// The backend type a TAC `Value` naturally carries: literals get their
/// obvious type, and named values (`Var`/`Temp`) fall back to `Int64` if
/// they aren't in `var_types` for some reason (shouldn't normally happen).
fn value_backend_type(value: &Value, var_types: &ScopedMap) -> BackendType {
    match value {
        Value::Var(name) | Value::Temp(name) => var_types
            .get(name)
            .map(BackendType::from_frontend)
            .unwrap_or(BackendType::Int64),
        Value::Char(_) => BackendType::Char,
        Value::Const(_) => BackendType::Int64,
        Value::Bool(_) => BackendType::Bool,
        Value::Str(_) => BackendType::Ptr,
        Value::Void => BackendType::Int64,
    }
}

/// Looks up the Cranelift `Variable` bound to `name`, declaring a fresh one
/// (with type `ty`) the first time it's seen. `var_idx` hands out the next
/// free `Variable` slot.
fn get_or_create_var(
    builder: &mut FunctionBuilder,
    var_map: &mut HashMap<String, Variable>,
    var_idx: &mut usize,
    name: &str,
    ty: BackendType,
    ptr_type: cranelift::prelude::Type,
) -> Variable {
    if let Some(&v) = var_map.get(name) {
        return v;
    }
    let v = Variable::new(*var_idx);
    *var_idx += 1;
    builder.declare_var(v, ty.to_clif_type(ptr_type));
    var_map.insert(name.to_string(), v);
    v
}

fn get_or_create_block(
    builder: &mut FunctionBuilder,
    block_map: &mut HashMap<String, Block>,
    all_blocks: &mut Vec<Block>,
    name: &str,
) -> Block {
    if let Some(&blk) = block_map.get(name) {
        return blk;
    }
    let blk = builder.create_block();
    block_map.insert(name.to_string(), blk);
    all_blocks.push(blk);
    blk
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    Int8,
    Int32,
    Int64,
    UInt8,
    UInt32,
    UInt64,
    Char,
    Bool,
    Ptr,
}

impl BackendType {
    pub fn to_clif_type(&self, ptr_type: cranelift::prelude::Type) -> cranelift::prelude::Type {
        match self {
            BackendType::Char => types::I8,
            BackendType::Bool => types::I8,
            BackendType::Int8 | BackendType::UInt8 => types::I8,
            BackendType::Int32 | BackendType::UInt32 => types::I32,
            BackendType::Int64 | BackendType::UInt64 => types::I64,
            BackendType::Ptr => ptr_type,
        }
    }

    pub fn byte_size(&self) -> u32 {
        match self {
            BackendType::Char => 1,
            BackendType::Bool => 1,
            BackendType::Int8 | BackendType::UInt8 => 1,
            BackendType::Int32 | BackendType::UInt32 => 4,
            BackendType::Int64 | BackendType::UInt64 => 8,
            BackendType::Ptr => 8,
        }
    }

    pub fn from_frontend(ty: &Type) -> Self {
        match ty {
            Type::Str => BackendType::Ptr,
            Type::GenericInstance { name, .. } => {
                unreachable!(
                    "{}",
                    format!(
                        "generic instances '{}' must be monomorphised before codegen",
                        name
                    )
                )
            }
            Type::GenericParam(s) => {
                unreachable!(
                    "{}",
                    format!(
                        "generic parameters '{}' must be monomorphised before codegen",
                        s
                    )
                )
            }
            Type::Any => BackendType::Ptr,
            Type::Int => BackendType::Int64,
            Type::Int8 => BackendType::Int8,
            Type::UInt8 => BackendType::UInt8,
            Type::UInt => BackendType::UInt64,
            Type::Char => BackendType::Char,
            Type::Bool => BackendType::Bool,
            Type::Ptr(_) => BackendType::Ptr,
            Type::Struct(_) => BackendType::Ptr,
            Type::Void => BackendType::Int64,
            Type::Array { .. } => BackendType::Ptr,
        }
    }
}

enum AbiType {
    Void,
    Primitive(cranelift::prelude::Type),
    Aggregate { chunk_count: usize, total_size: u32 },
}

impl AbiType {
    fn from_frontend(
        ty: &Type,
        struct_defs: &HashMap<String, StructLayout>,
        ptr_type: cranelift::prelude::Type,
    ) -> Self {
        match ty {
            Type::Void => AbiType::Void,
            Type::Struct(name) => {
                let stripped = strip_mangling(name);
                let layout = struct_defs
                    .get(name)
                    .or_else(|| struct_defs.get(stripped))
                    .or_else(|| {
                        name.split("::")
                            .last()
                            .and_then(|suffix| struct_defs.get(suffix))
                    })
                    .or_else(|| {
                        stripped
                            .split("::")
                            .last()
                            .and_then(|suffix| struct_defs.get(suffix))
                    });

                if let Some(layout) = layout {
                    let total_size = layout.total_size;
                    let chunk_count = ((total_size + 7) / 8) as usize;
                    AbiType::Aggregate {
                        chunk_count,
                        total_size: total_size.try_into().unwrap(),
                    }
                } else {
                    AbiType::Primitive(ptr_type)
                }
            }
            _ => AbiType::Primitive(BackendType::from_frontend(ty).to_clif_type(ptr_type)),
        }
    }

    fn append_to_signature_returns(&self, sig: &mut Signature) {
        match self {
            Self::Void => {}
            Self::Primitive(clif_ty) => {
                sig.returns.push(AbiParam::new(*clif_ty));
            }
            Self::Aggregate { chunk_count, .. } => {
                for _ in 0..*chunk_count {
                    sig.returns.push(AbiParam::new(types::I64));
                }
            }
        }
    }
}

pub struct CraneliftBackend {
    pub module: ObjectModule,
    pub struct_defs: HashMap<String, StructLayout>,
    pub functions: HashMap<String, FunctionSignature>,
    pub string_literals: HashMap<String, DataId>,
    pub declared_funcs: HashMap<String, FuncId>,
}

impl CraneliftBackend {
    pub fn new(
        struct_defs: HashMap<String, StructLayout>,
        functions: HashMap<String, FunctionSignature>,
    ) -> Self {
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "true").unwrap();
        let isa_builder = cranelift_native::builder().unwrap_or_else(|msg| {
            panic!("host machine is not supported: {}", msg);
        });
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder = ObjectBuilder::new(
            isa,
            "mysz_output",
            cranelift_module::default_libcall_names(),
        )
        .unwrap();
        let module = ObjectModule::new(builder);

        Self {
            module,
            struct_defs,
            functions,
            string_literals: HashMap::new(),
            declared_funcs: HashMap::new(),
        }
    }

    pub fn scan_externs(&mut self, insts: &[Instruction]) {
        for inst in insts {
            if let Instruction::Extern { fnname } = inst {
                let s_name = strip_mangling(fnname);
                let linkage = Linkage::Import;

                let mut sig = self.module.make_signature();
                if let Some(func_sig) = self
                    .functions
                    .get(fnname)
                    .or_else(|| self.functions.get(strip_mangling(fnname)))
                {
                    let ptr_type = self.module.target_config().pointer_type();
                    for param_ty in &func_sig.param_types {
                        let backend_ty = BackendType::from_frontend(param_ty);
                        sig.params
                            .push(AbiParam::new(backend_ty.to_clif_type(ptr_type)));
                    }
                    let abi =
                        AbiType::from_frontend(&func_sig.return_type, &self.struct_defs, ptr_type);
                    abi.append_to_signature_returns(&mut sig);
                } else {
                    sig.returns.push(AbiParam::new(types::I64));
                }

                let fn_id = self.module.declare_function(s_name, linkage, &sig).unwrap();
                self.declared_funcs.insert(fnname.clone(), fn_id);
            }
        }
    }

    pub fn pre_declare_strings(&mut self, insts: &[&Instruction]) {
        let mut idx = 0;
        for inst in insts {
            if let Instruction::Arg {
                value: Value::Str(text),
            } = inst
            {
                if !self.string_literals.contains_key(text) {
                    let name = format!("_str_lit_{}", idx);
                    idx += 1;
                    let data_id = self
                        .module
                        .declare_data(&name, Linkage::Local, false, false)
                        .unwrap();
                    let mut desc = DataDescription::new();
                    let mut bytes = text.as_bytes().to_vec();
                    bytes.push(0);
                    desc.define(bytes.into_boxed_slice());
                    self.module.define_data(data_id, &desc).unwrap();
                    self.string_literals.insert(text.clone(), data_id);
                }
            }
        }
    }

    fn declare_string_literal(&mut self, text: &str) -> DataId {
        if let Some(&id) = self.string_literals.get(text) {
            id
        } else {
            let idx = self.string_literals.len();
            let name = format!("_str_lit_{}", idx);
            let data_id = self
                .module
                .declare_data(&name, Linkage::Local, false, false)
                .unwrap();
            let mut desc = DataDescription::new();
            let mut bytes = text.as_bytes().to_vec();
            bytes.push(0);
            desc.define(bytes.into_boxed_slice());
            self.module.define_data(data_id, &desc).unwrap();
            self.string_literals.insert(text.to_string(), data_id);
            data_id
        }
    }

    fn get_or_declare_func(
        &mut self,
        name: &str,
        public: bool,
        param_types: &[BackendType],
        return_type: Option<&Type>,
    ) -> FuncId {
        if let Some(&id) = self.declared_funcs.get(name) {
            return id;
        }

        let s_name = name;

        let linkage = if public {
            Linkage::Export
        } else {
            Linkage::Local
        };

        let ptr_type = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();

        for ty in param_types {
            sig.params.push(AbiParam::new(ty.to_clif_type(ptr_type)));
        }

        if let Some(front_ret) = return_type {
            let abi = AbiType::from_frontend(front_ret, &self.struct_defs, ptr_type);
            abi.append_to_signature_returns(&mut sig);
        } else {
            sig.returns.push(AbiParam::new(types::I64));
        }

        let id = self.module.declare_function(s_name, linkage, &sig).unwrap();
        self.declared_funcs.insert(name.to_string(), id);
        id
    }

    /// Materialises a TAC `Value` as a Cranelift SSA value: constants become
    /// immediates, string literals become data references, and named values
    /// are either loaded from their stack slot (if they're an out-of-line aggregate) or read from their SSA variable.
    ///
    /// This is the single place that knows how to turn a `Value` into a
    /// `cranelift::prelude::Value`; every instruction handler below that
    /// needs to read an operand goes through here.
    fn lower_value(
        &mut self,
        builder: &mut FunctionBuilder,
        value: &Value,
        var_types: &ScopedMap,
        var_map: &mut HashMap<String, Variable>,
        var_idx: &mut usize,
        stack_slot_map: &HashMap<String, StackSlot>,
        ptr_type: cranelift::prelude::Type,
    ) -> cranelift::prelude::Value {
        let ty = value_backend_type(value, var_types).to_clif_type(ptr_type);

        match value {
            Value::Const(n) => builder.ins().iconst(ty, *n),
            Value::Bool(b) => builder.ins().iconst(ty, if *b { 1 } else { 0 }),
            Value::Void => builder.ins().iconst(ty, 0),
            Value::Char(ch) => {
                let mut buffer = [0; 4];
                let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                builder.ins().iconst(ty, byte_val as i64)
            }
            Value::Str(text) => {
                let data_id = self.declare_string_literal(text);
                let local_ref = self.module.declare_data_in_func(data_id, &mut builder.func);
                builder.ins().global_value(ty, local_ref)
            }
            Value::Var(name) | Value::Temp(name) => {
                if let Some(&slot) = stack_slot_map.get(name) {
                    builder.ins().stack_load(ty, slot, 0)
                } else {
                    let var_ty = value_backend_type(value, var_types);
                    let v = get_or_create_var(builder, var_map, var_idx, name, var_ty, ptr_type);
                    builder.use_var(v)
                }
            }
        }
    }

    pub fn compile_function(
        &mut self,
        name: &str,
        public: bool,
        insts: &[&Instruction],
        ctx: &mut Context,
        func_ctx: &mut FunctionBuilderContext,
        incoming_var_types: &ScopedMap,
    ) {
        let mut terminated = false;

        let ptr_type = self.module.target_config().pointer_type();
        let mut var_types = incoming_var_types.clone();
        var_types.push_scope();

        let mut param_idx = 0;
        for inst in insts {
            if let Instruction::Param { p } = inst {
                let resolved_ty = if let Some(func_sig) = self
                    .functions
                    .get(name)
                    .or_else(|| self.functions.get(strip_mangling(name)))
                {
                    if let Some(formal_ty) = func_sig.param_types.get(param_idx) {
                        formal_ty.clone()
                    } else {
                        incoming_var_types.get(p).cloned().unwrap_or(Type::Int)
                    }
                } else {
                    incoming_var_types.get(p).cloned().unwrap_or(Type::Int)
                };
                var_types.insert(p.clone(), resolved_ty);
                param_idx += 1;
            }
        }

        let current_func_ret_front = self
            .functions
            .get(name)
            .or_else(|| self.functions.get(strip_mangling(name)))
            .map(|sig| sig.return_type.clone());

        let mut sig = self.module.make_signature();
        if let Some(ref front_ret) = current_func_ret_front {
            let abi = AbiType::from_frontend(front_ret, &self.struct_defs, ptr_type);
            abi.append_to_signature_returns(&mut sig);
        } else {
            sig.returns.push(AbiParam::new(types::I64));
        }

        let mut param_backend_types: Vec<BackendType> = Vec::new();
        let mut param_idx = 0;

        for inst in insts {
            if let Instruction::Param { p } = inst {
                let ty = if let Some(func_sig) = self
                    .functions
                    .get(name)
                    .or_else(|| self.functions.get(strip_mangling(name)))
                {
                    if let Some(formal_ty) = func_sig.param_types.get(param_idx) {
                        BackendType::from_frontend(formal_ty)
                    } else {
                        var_types
                            .get(p)
                            .map(BackendType::from_frontend)
                            .unwrap_or(BackendType::Int64)
                    }
                } else {
                    var_types
                        .get(p)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64)
                };

                sig.params.push(AbiParam::new(ty.to_clif_type(ptr_type)));
                param_backend_types.push(ty);
                param_idx += 1;
            }
        }

        let func_id = self.get_or_declare_func(
            name,
            public,
            &param_backend_types,
            current_func_ret_front.as_ref(),
        );
        self.declared_funcs.insert(name.to_string(), func_id);
        ctx.func.signature = sig;

        let mut builder = FunctionBuilder::new(&mut ctx.func, func_ctx);
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);

        let mut var_map: HashMap<String, Variable> = HashMap::new();
        let mut stack_slot_map: HashMap<String, StackSlot> = HashMap::new();
        let mut var_idx = 0;

        let mut block_map: HashMap<String, Block> = HashMap::new();

        for inst in insts {
            if let Instruction::Param { p } = inst {
                let frontend_type = var_types
                    .get(p)
                    .or_else(|| {
                        let combined = format!("{}::{}", name, p);
                        var_types.get(&combined)
                    })
                    .or_else(|| {
                        p.split("::")
                            .last()
                            .and_then(|suffix| var_types.get(suffix))
                    });

                if let Some(frontend_type) = frontend_type {
                    let abi = AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                    if let AbiType::Aggregate { total_size, .. } = abi {
                        if !stack_slot_map.contains_key(p) {
                            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                StackSlotKind::ExplicitSlot,
                                total_size,
                            ));
                            stack_slot_map.insert(p.clone(), slot);
                        }
                    }
                }
            }
            if let Instruction::Store { ptr, source } = inst {
                for val in [ptr, source] {
                    if let Value::Var(var_name) | Value::Temp(var_name) = val {
                        let frontend_type = var_types
                            .get(var_name)
                            .or_else(|| {
                                let combined = format!("{}::{}", name, var_name);
                                var_types.get(&combined)
                            })
                            .or_else(|| {
                                var_name
                                    .split("::")
                                    .last()
                                    .and_then(|suffix| var_types.get(suffix))
                            });

                        if let Some(frontend_type) = frontend_type {
                            let abi =
                                AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                            if let AbiType::Aggregate { total_size, .. } = abi {
                                if !stack_slot_map.contains_key(var_name) {
                                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                        StackSlotKind::ExplicitSlot,
                                        total_size,
                                    ));
                                    stack_slot_map.insert(var_name.clone(), slot);
                                }
                            }
                        }
                    }
                }
            }
            if let Instruction::Assign { dst: dest_name, .. }
            | Instruction::Load { dst: dest_name, .. }
            | Instruction::Binary { dst: dest_name, .. }
            | Instruction::Unary { dst: dest_name, .. } = inst
            {
                let frontend_type = var_types
                    .get(dest_name)
                    .or_else(|| {
                        let combined = format!("{}::{}", name, dest_name);
                        var_types.get(&combined)
                    })
                    .or_else(|| {
                        dest_name
                            .split("::")
                            .last()
                            .and_then(|suffix| var_types.get(suffix))
                    });

                if let Some(frontend_type) = frontend_type {
                    let abi = AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                    if let AbiType::Aggregate { total_size, .. } = abi {
                        if !stack_slot_map.contains_key(dest_name) {
                            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                StackSlotKind::ExplicitSlot,
                                total_size,
                            ));
                            stack_slot_map.insert(dest_name.clone(), slot);
                        }
                    }
                }
            }
            if let Instruction::Unary {
                dst: dest_name,
                op,
                value: src,
            } = inst
            {
                let f_ty = var_types
                    .get(dest_name)
                    .or_else(|| {
                        let combined = format!("{}::{}", name, dest_name);
                        var_types.get(&combined)
                    })
                    .or_else(|| {
                        dest_name
                            .split("::")
                            .last()
                            .and_then(|suffix| var_types.get(suffix))
                    });

                if let Some(frontend_type) = f_ty {
                    let abi = AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                    if let AbiType::Aggregate { total_size, .. } = abi {
                        if !stack_slot_map.contains_key(dest_name) {
                            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                StackSlotKind::ExplicitSlot,
                                total_size,
                            ));
                            stack_slot_map.insert(dest_name.clone(), slot);
                        }
                    }
                }

                if *op == IrOp::Ref {
                    if let Value::Var(src_name) | Value::Temp(src_name) = src {
                        let src_front_ty = var_types
                            .get(src_name)
                            .or_else(|| {
                                let combined = format!("{}::{}", name, src_name);
                                var_types.get(&combined)
                            })
                            .or_else(|| {
                                src_name
                                    .split("::")
                                    .last()
                                    .and_then(|suffix| var_types.get(suffix))
                            });

                        if let Some(src_front_ty) = src_front_ty {
                            let abi =
                                AbiType::from_frontend(src_front_ty, &self.struct_defs, ptr_type);

                            let size = match abi {
                                AbiType::Aggregate { total_size, .. } => total_size,
                                _ => BackendType::from_frontend(src_front_ty).byte_size(),
                            };

                            if !stack_slot_map.contains_key(src_name) {
                                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                    StackSlotKind::ExplicitSlot,
                                    size,
                                ));
                                stack_slot_map.insert(src_name.clone(), slot);
                            }
                        }
                    }
                }
            }

            if let Instruction::Binary { dst: dest_name, .. }
            | Instruction::Load { dst: dest_name, .. }
            | Instruction::Assign { dst: dest_name, .. } = inst
            {
                let f_ty = var_types
                    .get(dest_name)
                    .or_else(|| {
                        let combined = format!("{}::{}", name, dest_name);
                        var_types.get(&combined)
                    })
                    .or_else(|| {
                        dest_name
                            .split("::")
                            .last()
                            .and_then(|suffix| var_types.get(suffix))
                    });

                if let Some(frontend_type) = f_ty {
                    let abi = AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                    if let AbiType::Aggregate { total_size, .. } = abi {
                        if !stack_slot_map.contains_key(dest_name) {
                            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                StackSlotKind::ExplicitSlot,
                                total_size,
                            ));
                            stack_slot_map.insert(dest_name.clone(), slot);
                        }
                    }
                }
            }
            if let Instruction::Call {
                dest: Some(dest_name),
                ..
            } = inst
            {
                let frontend_type = var_types
                    .get(dest_name)
                    .or_else(|| {
                        let combined = format!("{}::{}", name, dest_name);
                        var_types.get(&combined)
                    })
                    .or_else(|| {
                        dest_name
                            .split("::")
                            .last()
                            .and_then(|suffix| var_types.get(suffix))
                    });

                if let Some(frontend_type) = frontend_type {
                    let abi = AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                    if let AbiType::Aggregate { total_size, .. } = abi {
                        if !stack_slot_map.contains_key(dest_name) {
                            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                StackSlotKind::ExplicitSlot,
                                total_size,
                            ));
                            stack_slot_map.insert(dest_name.clone(), slot);
                        }
                    }
                }
            }
        }

        let mut all_blocks = Vec::new();
        all_blocks.push(entry_block);

        let mut current_param_idx = 0;
        for inst in insts {
            if let Instruction::Param { p } = inst {
                let arg_val = builder.block_params(entry_block)[current_param_idx];
                current_param_idx += 1;

                let ty = var_types
                    .get(p)
                    .map(BackendType::from_frontend)
                    .unwrap_or(BackendType::Int64);
                let frontend_type = var_types.get(p).unwrap_or(&Type::Int);
                let abi = AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);

                if let AbiType::Aggregate { total_size, .. } = abi {
                    let slot = *stack_slot_map.get(p).unwrap();
                    let addr = builder.ins().stack_addr(ptr_type, slot, 0);
                    let size_val = builder.ins().iconst(ptr_type, total_size as i64);
                    builder.call_memcpy(self.module.target_config(), addr, arg_val, size_val);
                } else {
                    let v = get_or_create_var(
                        &mut builder,
                        &mut var_map,
                        &mut var_idx,
                        p,
                        ty,
                        ptr_type,
                    );
                    builder.def_var(v, arg_val);
                }
            }
        }

        for (var_name, &slot) in &stack_slot_map {
            let is_param = insts.iter().any(|inst| {
                if let Instruction::Param { p } = inst {
                    p == var_name
                } else {
                    false
                }
            });
            if is_param {
                continue;
            }

            let frontend_type = var_types
                .get(var_name)
                .or_else(|| {
                    let combined = format!("{}::{}", name, var_name);
                    var_types.get(&combined)
                })
                .or_else(|| {
                    var_name
                        .split("::")
                        .last()
                        .and_then(|suffix| var_types.get(suffix))
                });

            if let Some(frontend_type) = frontend_type {
                let abi = AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                if let AbiType::Aggregate { total_size, .. } = abi {
                    let mut offset = 0;
                    while offset < total_size {
                        let remaining = total_size - offset;
                        if remaining >= 8 {
                            let zero = builder.ins().iconst(types::I64, 0);
                            builder.ins().stack_store(zero, slot, offset as i32);
                            offset += 8;
                        } else if remaining >= 4 {
                            let zero = builder.ins().iconst(types::I32, 0);
                            builder.ins().stack_store(zero, slot, offset as i32);
                            offset += 4;
                        } else if remaining >= 2 {
                            let zero = builder.ins().iconst(types::I16, 0);
                            builder.ins().stack_store(zero, slot, offset as i32);
                            offset += 2;
                        } else {
                            let zero = builder.ins().iconst(types::I8, 0);
                            builder.ins().stack_store(zero, slot, offset as i32);
                            offset += 1;
                        }
                    }
                }
            }
        }

        let mut call_args: Vec<cranelift::prelude::Value> = Vec::new();
        let mut call_arg_types: Vec<BackendType> = Vec::new();

        let mut param_index = 0;

        for inst in insts {
            if terminated && !matches!(inst, Instruction::Label(_)) {
                continue;
            }

            match inst {
                Instruction::FunctionLabel(_) | Instruction::Extern { .. } => {}
                Instruction::Label(lbl_name) => {
                    let blk = get_or_create_block(
                        &mut builder,
                        &mut block_map,
                        &mut all_blocks,
                        lbl_name,
                    );
                    all_blocks.push(blk);

                    let current_blk = builder.current_block();
                    let needs_jump = if let Some(curr) = current_blk {
                        match builder.func.layout.last_inst(curr) {
                            Some(last_inst) => {
                                !builder.func.dfg.insts[last_inst].opcode().is_terminator()
                            }
                            None => true,
                        }
                    } else {
                        false
                    };

                    if needs_jump {
                        builder.ins().jump(blk, &[]);
                    }
                    builder.switch_to_block(blk);
                    terminated = false;
                }
                Instruction::Param { p } => {
                    let arg_val = builder.block_params(entry_block)[param_index];

                    let slot = stack_slot_map.get(p).or_else(|| {
                        let local_name = p.split("::").last().unwrap_or(p);
                        stack_slot_map.get(local_name)
                    });

                    if let Some(&s) = slot {
                        builder.ins().stack_store(arg_val, s, 0);
                    } else {
                        let dest_ty =
                            BackendType::from_frontend(var_types.get(p).unwrap_or(&Type::Int));
                        let var_id = get_or_create_var(
                            &mut builder,
                            &mut var_map,
                            &mut var_idx,
                            p,
                            dest_ty,
                            ptr_type,
                        );
                        builder.def_var(var_id, arg_val);
                    }

                    param_index += 1;
                }
                Instruction::Arg { value } => {
                    if let Value::Var(var_name) | Value::Temp(var_name) = value {
                        let frontend_type = var_types
                            .get(var_name)
                            .or_else(|| {
                                let combined = format!("{}::{}", name, var_name);
                                var_types.get(&combined)
                            })
                            .or_else(|| {
                                var_name
                                    .split("::")
                                    .last()
                                    .and_then(|suffix| var_types.get(suffix))
                            });

                        if let Some(frontend_type) = frontend_type {
                            let abi =
                                AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                            if let AbiType::Aggregate { .. } = abi {
                                let slot = *stack_slot_map
                                    .get(var_name)
                                    .or_else(|| {
                                        let combined = format!("{}::{}", name, var_name);
                                        stack_slot_map.get(&combined)
                                    })
                                    .or_else(|| {
                                        var_name
                                            .split("::")
                                            .last()
                                            .and_then(|suffix| stack_slot_map.get(suffix))
                                    })
                                    .unwrap_or_else(|| {
                                        panic!("Arg stack slot not found for: {}", var_name)
                                    });

                                let addr_val = builder.ins().stack_addr(ptr_type, slot, 0);
                                call_args.push(addr_val);
                                call_arg_types.push(BackendType::Ptr);
                                continue;
                            }
                        }
                    }

                    let arg_ty = value_backend_type(value, &var_types);
                    let val = self.lower_value(
                        &mut builder,
                        value,
                        &var_types,
                        &mut var_map,
                        &mut var_idx,
                        &stack_slot_map,
                        ptr_type,
                    );

                    call_args.push(val);
                    call_arg_types.push(arg_ty);
                }
                Instruction::Cast {
                    dst: dest_name,
                    cast_ty,
                    value,
                    to_type,
                } => {
                    let source_val = self.lower_value(
                        &mut builder,
                        value,
                        &var_types,
                        &mut var_map,
                        &mut var_idx,
                        &stack_slot_map,
                        ptr_type,
                    );

                    let dest_backend_ty = BackendType::from_frontend(&to_type);
                    let clif_target_ty = dest_backend_ty.to_clif_type(ptr_type);

                    let casted_val = match cast_ty {
                        CastType::BitCast => {
                            // A bitcast reinterprets the bits without changing them
                            builder
                                .ins()
                                .bitcast(clif_target_ty, MemFlags::new(), source_val)
                        }
                        CastType::Extend => {
                            // If signed use ireduce/sextend. For safety with generic ints,
                            // standard zero/sign extension depending on signedness layout:
                            // Assuming unsigned/zero-extension default here:
                            builder.ins().uextend(clif_target_ty, source_val)
                        }
                        CastType::Truncate => {
                            // High bits are chopped off
                            builder.ins().ireduce(clif_target_ty, source_val)
                        }
                    };

                    if let Some(&slot) = stack_slot_map.get(dest_name) {
                        builder.ins().stack_store(casted_val, slot, 0);
                    } else {
                        let v_dest = get_or_create_var(
                            &mut builder,
                            &mut var_map,
                            &mut var_idx,
                            dest_name,
                            dest_backend_ty,
                            ptr_type,
                        );
                        builder.def_var(v_dest, casted_val);
                    }
                }
                Instruction::Call {
                    name: callee_name,
                    dest,
                    ..
                } => {
                    let mut final_sig = self.module.make_signature();
                    for ty in &call_arg_types {
                        final_sig
                            .params
                            .push(AbiParam::new(ty.to_clif_type(ptr_type)));
                    }

                    let callee_ret_front = self
                        .functions
                        .get(callee_name)
                        .or_else(|| self.functions.get(strip_mangling(callee_name)))
                        .map(|sig| sig.return_type.clone());

                    if let Some(ref front_ret) = callee_ret_front {
                        let abi = AbiType::from_frontend(front_ret, &self.struct_defs, ptr_type);
                        abi.append_to_signature_returns(&mut final_sig);
                    } else if let Some(d_name) = dest {
                        if let Some(front_ret) = var_types.get(d_name) {
                            let abi =
                                AbiType::from_frontend(front_ret, &self.struct_defs, ptr_type);
                            abi.append_to_signature_returns(&mut final_sig);
                        } else {
                            final_sig.returns.push(AbiParam::new(types::I64));
                        }
                    } else {
                        final_sig.returns.push(AbiParam::new(types::I64));
                    }

                    let fn_id = if let Some(&id) = self.declared_funcs.get(callee_name) {
                        id
                    } else {
                        let stripped = strip_mangling(callee_name);
                        if let Some(&id) = self.declared_funcs.get(stripped) {
                            id
                        } else {
                            let linkage = if callee_name == "main" {
                                Linkage::Export
                            } else {
                                Linkage::Local
                            };

                            let id = self
                                .module
                                .declare_function(callee_name, linkage, &final_sig)
                                .unwrap();

                            self.declared_funcs.insert(callee_name.to_string(), id);
                            id
                        }
                    };

                    let local_func = self.module.declare_func_in_func(fn_id, &mut builder.func);

                    let inst_call = builder.ins().call(local_func, &call_args);
                    let call_results = builder.inst_results(inst_call).to_vec();

                    if let Some(dest_name) = dest {
                        let frontend_type = var_types
                            .get(dest_name)
                            .or_else(|| {
                                let combined = format!("{}::{}", name, dest_name);
                                var_types.get(&combined)
                            })
                            .or_else(|| {
                                dest_name
                                    .split("::")
                                    .last()
                                    .and_then(|suffix| var_types.get(suffix))
                            })
                            .unwrap_or(&Type::Int);
                        let abi =
                            AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);

                        if let AbiType::Aggregate {
                            chunk_count,
                            total_size,
                        } = abi
                        {
                            if !stack_slot_map.contains_key(dest_name) {
                                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                    StackSlotKind::ExplicitSlot,
                                    total_size,
                                ));
                                stack_slot_map.insert(dest_name.clone(), slot);
                            }
                            let slot = *stack_slot_map.get(dest_name).unwrap();
                            for i in 0..chunk_count {
                                let val_part = call_results[i];
                                builder.ins().stack_store(val_part, slot, (i * 8) as i32);
                            }
                        } else {
                            let dest_ty = BackendType::from_frontend(frontend_type);
                            if !call_results.is_empty() {
                                let res_val = call_results[0];
                                if let Some(&slot) = stack_slot_map.get(dest_name) {
                                    builder.ins().stack_store(res_val, slot, 0);
                                } else {
                                    let v = get_or_create_var(
                                        &mut builder,
                                        &mut var_map,
                                        &mut var_idx,
                                        dest_name,
                                        dest_ty,
                                        ptr_type,
                                    );
                                    builder.def_var(v, res_val);
                                }
                            }
                        }
                    }
                    call_args.clear();
                    call_arg_types.clear();
                }

                Instruction::Assign {
                    dst: dst_name,
                    src: value,
                } => {
                    let frontend_type = var_types
                        .get(dst_name)
                        .or_else(|| {
                            let combined = format!("{}::{}", name, dst_name);
                            var_types.get(&combined)
                        })
                        .or_else(|| {
                            dst_name
                                .split("::")
                                .last()
                                .and_then(|suffix| var_types.get(suffix))
                        })
                        .unwrap_or(&Type::Int);
                    let abi = AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);

                    if let AbiType::Aggregate {
                        total_size,
                        chunk_count,
                    } = abi
                    {
                        let dest_slot = *stack_slot_map
                            .get(dst_name)
                            .or_else(|| {
                                let combined = format!("{}::{}", name, dst_name);
                                stack_slot_map.get(&combined)
                            })
                            .or_else(|| {
                                dst_name
                                    .split("::")
                                    .last()
                                    .and_then(|suffix| stack_slot_map.get(suffix))
                            })
                            .unwrap_or_else(|| {
                                panic!("Destination block space unallocated: {}", dst_name)
                            });

                        let dest_addr = builder.ins().stack_addr(ptr_type, dest_slot, 0);

                        match value {
                            Value::Var(src_name) | Value::Temp(src_name) => {
                                let src_slot = stack_slot_map.get(src_name)
                                    .copied()
                                    .or_else(|| {
                                        let combined_prefix = format!("{}::{}", name, src_name);
                                        stack_slot_map.get(&combined_prefix).copied()
                                    })
                                    .or_else(|| {
                                        src_name.split("::").last().and_then(|suffix| stack_slot_map.get(suffix).copied())
                                    })
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "Source block space unallocated. Tried: '{}', '{}::{}', and '{}'", 
                                            src_name, name, src_name, src_name.split("::").last().unwrap_or("")
                                        )
                                    });

                                let src_addr = builder.ins().stack_addr(ptr_type, src_slot, 0);
                                let size_val = builder.ins().iconst(ptr_type, total_size as i64);
                                builder.call_memcpy(
                                    self.module.target_config(),
                                    dest_addr,
                                    src_addr,
                                    size_val,
                                );
                            }
                            Value::Const(0) => {
                                let zero_val = builder.ins().iconst(types::I64, 0);
                                for i in 0..chunk_count {
                                    builder
                                        .ins()
                                        .stack_store(zero_val, dest_slot, (i * 8) as i32);
                                }
                            }
                            _ => panic!("Direct block assignment from literals is unsupported."),
                        }
                    } else {
                        let dest_ty = BackendType::from_frontend(frontend_type);

                        let val = self.lower_value(
                            &mut builder,
                            value,
                            &var_types,
                            &mut var_map,
                            &mut var_idx,
                            &stack_slot_map,
                            ptr_type,
                        );
                        if let Some(&slot) = stack_slot_map.get(dst_name) {
                            builder.ins().stack_store(val, slot, 0);
                        } else {
                            let v_dest = get_or_create_var(
                                &mut builder,
                                &mut var_map,
                                &mut var_idx,
                                dst_name,
                                dest_ty,
                                ptr_type,
                            );
                            builder.def_var(v_dest, val);
                        }
                    }
                }
                Instruction::Load {
                    dst: dest_name,
                    ptr,
                    ..
                } => {
                    let ptr_val = self.lower_value(
                        &mut builder,
                        ptr,
                        &var_types,
                        &mut var_map,
                        &mut var_idx,
                        &stack_slot_map,
                        ptr_type,
                    );

                    let dest_ty = var_types
                        .get(dest_name)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);

                    let mut handled_as_aggregate = false;
                    let dest_f_ty = var_types.get(dest_name);

                    if let Some(frontend_type) = dest_f_ty {
                        let abi =
                            AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                        if let AbiType::Aggregate { total_size, .. } = abi {
                            if let Some(&dest_slot) = stack_slot_map.get(dest_name) {
                                let dest_addr = builder.ins().stack_addr(ptr_type, dest_slot, 0);
                                let size_val = builder.ins().iconst(ptr_type, total_size as i64);

                                builder.call_memcpy(
                                    self.module.target_config(),
                                    dest_addr,
                                    ptr_val,
                                    size_val,
                                );
                                handled_as_aggregate = true;
                            }
                        }
                    }

                    if !handled_as_aggregate {
                        let clif_ty = dest_ty.to_clif_type(ptr_type);
                        let loaded_val = builder.ins().load(clif_ty, MemFlags::new(), ptr_val, 0);

                        if let Some(&slot) = stack_slot_map.get(dest_name) {
                            builder.ins().stack_store(loaded_val, slot, 0);
                        } else {
                            let v_dest = get_or_create_var(
                                &mut builder,
                                &mut var_map,
                                &mut var_idx,
                                dest_name,
                                dest_ty,
                                ptr_type,
                            );
                            builder.def_var(v_dest, loaded_val);
                        }
                    }
                }
                Instruction::Store { ptr, source: value } => {
                    let ptr_val = self.lower_value(
                        &mut builder,
                        ptr,
                        &var_types,
                        &mut var_map,
                        &mut var_idx,
                        &stack_slot_map,
                        ptr_type,
                    );

                    let mut handled_as_aggregate = false;
                    if let Value::Var(src_name) | Value::Temp(src_name) = value {
                        if let Some(frontend_type) = var_types.get(src_name) {
                            let abi =
                                AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                            if let AbiType::Aggregate { total_size, .. } = abi {
                                let slot = stack_slot_map
                                    .get(src_name)
                                    .copied()
                                    .or_else(|| {
                                        let combined = format!("{}::{}", src_name, src_name);
                                        stack_slot_map.get(&combined).copied()
                                    })
                                    .or_else(|| {
                                        src_name
                                            .split("::")
                                            .last()
                                            .and_then(|suffix| stack_slot_map.get(suffix).copied())
                                    });

                                let src_addr = if let Some(s) = slot {
                                    builder.ins().stack_addr(ptr_type, s, 0)
                                } else {
                                    self.lower_value(
                                        &mut builder,
                                        value,
                                        &var_types,
                                        &mut var_map,
                                        &mut var_idx,
                                        &stack_slot_map,
                                        ptr_type,
                                    )
                                };

                                let size_val = builder.ins().iconst(ptr_type, total_size as i64);
                                builder.call_memcpy(
                                    self.module.target_config(),
                                    ptr_val,
                                    src_addr,
                                    size_val,
                                );
                                handled_as_aggregate = true;
                            }
                        }
                    }

                    if !handled_as_aggregate {
                        let val = self.lower_value(
                            &mut builder,
                            value,
                            &var_types,
                            &mut var_map,
                            &mut var_idx,
                            &stack_slot_map,
                            ptr_type,
                        );
                        builder.ins().store(MemFlags::new(), val, ptr_val, 0);
                    }
                }
                Instruction::Binary {
                    dst: dest,
                    op,
                    lhs,
                    rhs,
                } => {
                    let dest_ty = var_types
                        .get(dest)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);
                    let v_dest = get_or_create_var(
                        &mut builder,
                        &mut var_map,
                        &mut var_idx,
                        dest,
                        dest_ty,
                        ptr_type,
                    );
                    let lhs_val = self.lower_value(
                        &mut builder,
                        lhs,
                        &var_types,
                        &mut var_map,
                        &mut var_idx,
                        &stack_slot_map,
                        ptr_type,
                    );
                    let rhs_val = self.lower_value(
                        &mut builder,
                        rhs,
                        &var_types,
                        &mut var_map,
                        &mut var_idx,
                        &stack_slot_map,
                        ptr_type,
                    );

                    let is_unsigned = matches!(dest_ty, BackendType::UInt32 | BackendType::UInt64)
                        || match lhs {
                            Value::Var(name) | Value::Temp(name) => {
                                if let Some(t) = var_types.get(name) {
                                    matches!(
                                        BackendType::from_frontend(t),
                                        BackendType::UInt32 | BackendType::UInt64
                                    )
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        };

                    let res = match op {
                        IrOp::Add => builder.ins().iadd(lhs_val, rhs_val),
                        IrOp::Sub => builder.ins().isub(lhs_val, rhs_val),
                        IrOp::Mul => builder.ins().imul(lhs_val, rhs_val),

                        IrOp::Div => {
                            if is_unsigned {
                                builder.ins().udiv(lhs_val, rhs_val)
                            } else {
                                builder.ins().sdiv(lhs_val, rhs_val)
                            }
                        }
                        IrOp::Mod => {
                            if is_unsigned {
                                builder.ins().urem(lhs_val, rhs_val)
                            } else {
                                builder.ins().srem(lhs_val, rhs_val)
                            }
                        }

                        IrOp::Eq => builder.ins().icmp(IntCC::Equal, lhs_val, rhs_val),
                        IrOp::NEq => builder.ins().icmp(IntCC::NotEqual, lhs_val, rhs_val),
                        IrOp::Lt => {
                            let cond = if is_unsigned {
                                IntCC::UnsignedLessThan
                            } else {
                                IntCC::SignedLessThan
                            };
                            builder.ins().icmp(cond, lhs_val, rhs_val)
                        }
                        IrOp::LtE => {
                            let cond = if is_unsigned {
                                IntCC::UnsignedLessThanOrEqual
                            } else {
                                IntCC::SignedLessThanOrEqual
                            };
                            builder.ins().icmp(cond, lhs_val, rhs_val)
                        }
                        IrOp::Gt => {
                            let cond = if is_unsigned {
                                IntCC::UnsignedGreaterThan
                            } else {
                                IntCC::SignedGreaterThan
                            };
                            builder.ins().icmp(cond, lhs_val, rhs_val)
                        }
                        IrOp::GtE => {
                            let cond = if is_unsigned {
                                IntCC::UnsignedGreaterThanOrEqual
                            } else {
                                IntCC::SignedGreaterThanOrEqual
                            };
                            builder.ins().icmp(cond, lhs_val, rhs_val)
                        }

                        IrOp::Neg | IrOp::Pos | IrOp::Ref | IrOp::Not => unreachable!(),
                    };

                    builder.def_var(v_dest, res);
                }
                Instruction::Unary {
                    dst: dest,
                    op,
                    value: src,
                } => {
                    let dest_ty = var_types
                        .get(dest)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);

                    let v_dest = get_or_create_var(
                        &mut builder,
                        &mut var_map,
                        &mut var_idx,
                        dest,
                        dest_ty,
                        ptr_type,
                    );

                    let src_val = self.lower_value(
                        &mut builder,
                        src,
                        &var_types,
                        &mut var_map,
                        &mut var_idx,
                        &stack_slot_map,
                        ptr_type,
                    );

                    let res = match op {
                        IrOp::Neg => builder.ins().ineg(src_val),
                        IrOp::Pos => src_val,
                        IrOp::Not => {
                            let is_zero = builder.ins().icmp_imm(
                                cranelift_codegen::ir::condcodes::IntCC::Equal,
                                src_val,
                                0,
                            );

                            let clif_ty = dest_ty.to_clif_type(ptr_type);
                            let one = builder.ins().iconst(clif_ty, 1);
                            let zero = builder.ins().iconst(clif_ty, 0);

                            builder.ins().select(is_zero, one, zero)
                        }
                        IrOp::Ref => {
                            if let Value::Var(name) | Value::Temp(name) = src {
                                let slot = stack_slot_map
                                    .get(name)
                                    .copied()
                                    .or_else(|| {
                                        let combined = format!("{}::{}", name, name);
                                        stack_slot_map.get(&combined).copied()
                                    })
                                    .or_else(|| {
                                        name.split("::")
                                            .last()
                                            .and_then(|suffix| stack_slot_map.get(suffix).copied())
                                    });

                                let slot = if let Some(s) = slot {
                                    s
                                } else {
                                    let src_front_ty =
                                        var_types.get(name).cloned().unwrap_or(Type::Int);
                                    let abi = AbiType::from_frontend(
                                        &src_front_ty,
                                        &self.struct_defs,
                                        ptr_type,
                                    );

                                    let size = match abi {
                                        AbiType::Aggregate { total_size, .. } => total_size,
                                        _ => BackendType::from_frontend(&src_front_ty).byte_size(),
                                    };

                                    let size = if size == 0 { 1 } else { size };

                                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                        StackSlotKind::ExplicitSlot,
                                        size as u32,
                                    ));

                                    let var_ty = value_backend_type(src, &var_types);
                                    let v = get_or_create_var(
                                        &mut builder,
                                        &mut var_map,
                                        &mut var_idx,
                                        name,
                                        var_ty,
                                        ptr_type,
                                    );
                                    let current_val = builder.use_var(v);

                                    builder.ins().stack_store(current_val, slot, 0);

                                    stack_slot_map.insert(name.clone(), slot);
                                    slot
                                };
                                builder.ins().stack_addr(ptr_type, slot, 0)
                            } else {
                                panic!("Cannot take a reference of a non-lvalue: {:?}", src);
                            }
                        }
                        _ => panic!("Unsupported unary structural instruction operation"),
                    };
                    if let Some(&slot) = stack_slot_map.get(dest) {
                        builder.ins().stack_store(res, slot, 0);
                    } else {
                        builder.def_var(v_dest, res);
                    }
                }
                Instruction::JumpIfFalse { cond, target } => {
                    let cond_val = self.lower_value(
                        &mut builder,
                        cond,
                        &var_types,
                        &mut var_map,
                        &mut var_idx,
                        &stack_slot_map,
                        ptr_type,
                    );

                    let f_blk =
                        get_or_create_block(&mut builder, &mut block_map, &mut all_blocks, target);
                    let next_blk = builder.create_block();
                    all_blocks.push(next_blk);

                    builder.ins().brif(cond_val, next_blk, &[], f_blk, &[]);
                    builder.switch_to_block(next_blk);
                    terminated = false;
                }
                Instruction::Jump(lbl) => {
                    let blk =
                        get_or_create_block(&mut builder, &mut block_map, &mut all_blocks, lbl);
                    builder.ins().jump(blk, &[]);
                    terminated = true;
                }
                Instruction::Return { value } => {
                    if let Some(front_ret) = &current_func_ret_front {
                        let abi = AbiType::from_frontend(front_ret, &self.struct_defs, ptr_type);
                        match abi {
                            AbiType::Void => {
                                builder.ins().return_(&[]);
                            }
                            AbiType::Primitive(clif_ty) => {
                                let val = match value {
                                    Value::Const(n) => builder.ins().iconst(clif_ty, *n),
                                    Value::Bool(b) => {
                                        builder.ins().iconst(clif_ty, if *b { 1 } else { 0 })
                                    }
                                    Value::Var(name) | Value::Temp(name) => {
                                        let ty = var_types
                                            .get(name)
                                            .map(BackendType::from_frontend)
                                            .unwrap_or(BackendType::Int64);
                                        let v = get_or_create_var(
                                            &mut builder,
                                            &mut var_map,
                                            &mut var_idx,
                                            name,
                                            ty,
                                            ptr_type,
                                        );
                                        builder.use_var(v)
                                    }
                                    Value::Void => builder.ins().iconst(clif_ty, 0),
                                    _ => panic!("Primitive unexpected literal type matching"),
                                };
                                builder.ins().return_(&[val]);
                            }
                            AbiType::Aggregate { chunk_count, .. } => {
                                if let Value::Var(name) | Value::Temp(name) = value {
                                    if !stack_slot_map.contains_key(name) {
                                        panic!(
                                            "Return aggregate slot missing for '{}' - ensure Call handler creates stack slot for aggregate results.",
                                            name
                                        );
                                    }
                                    let slot = *stack_slot_map.get(name).unwrap();
                                    let mut chunks = Vec::new();
                                    for i in 0..chunk_count {
                                        let val_part = builder.ins().stack_load(
                                            types::I64,
                                            slot,
                                            (i * 8) as i32,
                                        );
                                        chunks.push(val_part);
                                    }
                                    builder.ins().return_(&chunks);
                                } else {
                                    panic!(
                                        "Returning structural structures from raw primitive literal fields unhandled."
                                    );
                                }
                            }
                        }
                    } else {
                        builder.ins().return_(&[]);
                    }
                    terminated = true;
                }
            }
        }

        let default_ret_abi = current_func_ret_front
            .as_ref()
            .map(|front_ret| AbiType::from_frontend(front_ret, &self.struct_defs, ptr_type))
            .unwrap_or(AbiType::Primitive(types::I64));

        for &block in &all_blocks {
            let needs_fallback = match builder.func.layout.last_inst(block) {
                Some(last_inst) => !builder.func.dfg.insts[last_inst].opcode().is_terminator(),
                None => true,
            };
            if needs_fallback {
                builder.switch_to_block(block);
                let ret_vals: Vec<_> = match &default_ret_abi {
                    AbiType::Void => Vec::new(),
                    AbiType::Primitive(clif_ty) => {
                        vec![builder.ins().iconst(*clif_ty, 0)]
                    }
                    AbiType::Aggregate { chunk_count, .. } => (0..*chunk_count)
                        .map(|_| builder.ins().iconst(types::I64, 0))
                        .collect(),
                };
                builder.ins().return_(&ret_vals);
            }
        }

        let mut sealed_blocks = std::collections::HashSet::new();

        builder.seal_block(entry_block);
        sealed_blocks.insert(entry_block);

        for &block in &all_blocks {
            if sealed_blocks.insert(block) {
                builder.seal_block(block);
            }
        }

        builder.finalize();
        match self.module.define_function(func_id, ctx) {
            Ok(_) => {}
            Err(cranelift_module::ModuleError::DuplicateDefinition(fnname)) => {
                eprintln!(
                    "warning: function '{}' has been defined more than once, more recently seen definition has been ignored.",
                    fnname
                );
            }
            Err(e) => panic!("Failed to define function: {:?}", e),
        }
        ctx.clear();
    }

    pub fn finish(self) -> cranelift_object::ObjectProduct {
        self.module.finish()
    }
}
