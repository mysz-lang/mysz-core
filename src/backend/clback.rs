use std::collections::HashMap;

use cranelift::{codegen::ir::StackSlot, prelude::*};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::ir::ir::StructLayout;
use crate::ir::tac::{Instruction, IrOp, Value};
use crate::parse::parsing::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    Int32,
    Int64,
    Char,
    Bool,
    Ptr,
}

impl BackendType {
    pub fn to_clif_type(&self, ptr_type: cranelift::prelude::Type) -> cranelift::prelude::Type {
        match self {
            BackendType::Char => types::I8,
            BackendType::Bool => types::I8,
            BackendType::Int32 => types::I32,
            BackendType::Int64 => types::I64,
            BackendType::Ptr => ptr_type,
        }
    }

    pub fn byte_size(&self) -> u32 {
        match self {
            BackendType::Char => 1,
            BackendType::Bool => 1,
            BackendType::Int32 => 4,
            BackendType::Int64 => 8,
            BackendType::Ptr => 8,
        }
    }

    pub fn from_frontend(ty: &Type) -> Self {
        match ty {
            Type::Int => BackendType::Int64,
            Type::Bool => BackendType::Bool,
            Type::Char => BackendType::Char,
            Type::Str => BackendType::Ptr,
            Type::Ptr(_) => BackendType::Ptr,
            Type::Array { .. } => BackendType::Ptr,
            _ => BackendType::Int64,
        }
    }
}

pub enum AbiType {
    Void,
    Primitive(cranelift::prelude::Type),
    Aggregate { total_size: u32, chunk_count: u32 },
}

impl AbiType {
    pub fn from_frontend(
        ty: &Type,
        struct_defs: &HashMap<String, StructLayout>,
        ptr_type: cranelift::prelude::Type,
    ) -> Self {
        match ty {
            Type::Void => AbiType::Void,

            Type::Struct(name) => {
                let size = struct_defs
                    .get(name)
                    .map(|x| x.total_size as u32)
                    .unwrap_or(0);

                AbiType::Aggregate {
                    total_size: size,
                    chunk_count: (size + 7) / 8,
                }
            }

            Type::Array { element_type, size } => {
                let elem = BackendType::from_frontend(element_type).byte_size();
                let total = elem * (*size as u32);

                AbiType::Aggregate {
                    total_size: total,
                    chunk_count: (total + 7) / 8,
                }
            }

            other => AbiType::Primitive(BackendType::from_frontend(other).to_clif_type(ptr_type)),
        }
    }

    pub fn append_to_signature_returns(&self, sig: &mut cranelift::prelude::Signature) {
        match self {
            AbiType::Void => {}
            AbiType::Primitive(clif_ty) => {
                sig.returns.push(AbiParam::new(*clif_ty));
            }
            AbiType::Aggregate { chunk_count, .. } => {
                for _ in 0..*chunk_count {
                    sig.returns.push(AbiParam::new(types::I64));
                }
            }
        }
    }
}

pub struct CraneliftBackend {
    module: ObjectModule,
    extern_names: std::collections::HashSet<String>,
    declared_funcs: HashMap<String, FuncId>,
    str_count: usize,
    struct_defs: HashMap<String, StructLayout>,
    string_literals: HashMap<String, DataId>,
}

impl CraneliftBackend {
    pub fn new(struct_defs: HashMap<String, StructLayout>) -> Self {
        let mut flag_builder = settings::builder();
        flag_builder.set("is_pic", "true").unwrap();

        let isa_builder = cranelift_native::builder().unwrap();
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();

        let builder =
            ObjectBuilder::new(isa, "mysz_prog", cranelift_module::default_libcall_names())
                .unwrap();

        let module = ObjectModule::new(builder);
        Self {
            module,
            extern_names: std::collections::HashSet::new(),
            declared_funcs: HashMap::new(),
            str_count: 0,
            struct_defs,
            string_literals: HashMap::new(),
        }
    }

    pub fn finish(self) -> cranelift_object::ObjectProduct {
        self.module.finish()
    }

    pub fn scan_externs(&mut self, program: &[Instruction]) {
        for inst in program {
            if let Instruction::Extern { fnname } = inst {
                self.extern_names.insert(fnname.clone());
            }
        }
    }

    pub fn pre_declare_strings(&mut self, insts: &[&Instruction]) {
        for inst in insts {
            match inst {
                Instruction::Assign {
                    src: Value::Str(text),
                    ..
                } => {
                    self.declare_string_literal(text);
                }
                Instruction::Arg {
                    value: Value::Str(text),
                } => {
                    self.declare_string_literal(text);
                }
                Instruction::Load {
                    ptr: Value::Str(text),
                    ..
                } => {
                    self.declare_string_literal(text);
                }
                _ => {}
            }
        }
    }

    fn declare_string_literal(&mut self, text: &str) -> DataId {
        if let Some(id) = self.string_literals.get(text) {
            return *id;
        }

        let mut data_desc = DataDescription::new();
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        data_desc.define(bytes.into_boxed_slice());

        self.str_count += 1;
        let sym_name = format!("_str_lit_{}", self.str_count);

        let data_id = self
            .module
            .declare_data(&sym_name, Linkage::Local, false, false)
            .unwrap();
        self.module.define_data(data_id, &data_desc).unwrap();

        self.string_literals.insert(text.to_string(), data_id);
        data_id
    }

    fn get_or_declare_func(
        &mut self,
        name: &str,
        public: bool,
        arg_types: &[BackendType],
        frontend_return_type: Option<&Type>,
    ) -> FuncId {
        if let Some(id) = self.declared_funcs.get(name) {
            return *id;
        }

        let mut sig = self.module.make_signature();
        let ptr_type = self.module.target_config().pointer_type();

        for ty in arg_types {
            sig.params.push(AbiParam::new(ty.to_clif_type(ptr_type)));
        }

        match frontend_return_type {
            Some(Type::Void) | None => {}
            Some(other) => {
                let abi = AbiType::from_frontend(other, &self.struct_defs, ptr_type);
                abi.append_to_signature_returns(&mut sig);
            }
        }

        let linkage = if self.extern_names.contains(name) {
            Linkage::Import
        } else if public {
            Linkage::Export
        } else if name == "str_concat" {
            // well we already hard code str_concat in IR, no point not hard coding it here
            Linkage::Import
        } else {
            Linkage::Local
        };

        let id = self.module.declare_function(name, linkage, &sig).unwrap();

        self.declared_funcs.insert(name.to_string(), id);

        id
    }

    pub fn compile_function(
        &mut self,
        name: &str,
        public: bool,
        insts: &[&Instruction],
        ctx: &mut cranelift::codegen::Context,
        func_ctx: &mut FunctionBuilderContext,
        var_types: &HashMap<String, Type>,
    ) {
        let ptr_type = self.module.target_config().pointer_type();

        let get_val_backend_type = |val: &Value| -> BackendType {
            match val {
                Value::Var(n) | Value::Temp(n) => {
                    if let Some(t) = var_types.get(n) {
                        BackendType::from_frontend(t)
                    } else {
                        BackendType::Int64
                    }
                }
                Value::Char(_) => BackendType::Char,
                Value::Const(_) => BackendType::Int64,
                Value::Bool(_) => BackendType::Bool,
                Value::Str(_) => BackendType::Ptr,
                Value::Void => BackendType::Int64,
            }
        };

        let mut sig = self.module.make_signature();
        let current_func_ret_front = var_types.get(name);
        if let Some(front_ret) = current_func_ret_front {
            let abi = AbiType::from_frontend(front_ret, &self.struct_defs, ptr_type);
            abi.append_to_signature_returns(&mut sig);
        } else {
            sig.returns.push(AbiParam::new(types::I64));
        }

        for inst in insts {
            if let Instruction::Param { p } = inst {
                let ty = var_types
                    .get(p)
                    .map(BackendType::from_frontend)
                    .unwrap_or(BackendType::Int64);
                sig.params.push(AbiParam::new(ty.to_clif_type(ptr_type)));
            }
        }

        let func_id = self.get_or_declare_func(name, public, &[], current_func_ret_front);
        self.declared_funcs.insert(name.to_string(), func_id);
        ctx.func.signature = sig;

        let mut builder = FunctionBuilder::new(&mut ctx.func, func_ctx);
        let mut all_blocks: Vec<Block> = Vec::new();

        let entry_block = builder.create_block();
        all_blocks.push(entry_block);
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let mut var_map: HashMap<String, Variable> = HashMap::new();
        let mut stack_slot_map: HashMap<String, StackSlot> = HashMap::new();
        let mut label_map: HashMap<String, Block> = HashMap::new();
        let mut var_idx = 0;

        for inst in insts {
            let mut alloc_slot = |var_name: &String, force: bool| {
                if stack_slot_map.contains_key(var_name) {
                    return;
                }
                if let Some(frontend_type) = var_types.get(var_name) {
                    let abi = AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                    let size = match abi {
                        AbiType::Aggregate { total_size, .. } => Some(total_size),
                        _ if force => Some(BackendType::from_frontend(frontend_type).byte_size()),
                        _ => None,
                    };
                    if let Some(size) = size {
                        stack_slot_map.entry(var_name.clone()).or_insert_with(|| {
                            builder.create_sized_stack_slot(StackSlotData::new(
                                StackSlotKind::ExplicitSlot,
                                size,
                            ))
                        });
                    }
                }
            };
            match inst {
                Instruction::Assign { dst, .. } => alloc_slot(dst, false),
                Instruction::Param { p } => alloc_slot(p, false),
                Instruction::Load { dst, .. } => alloc_slot(dst, false),
                Instruction::Store {
                    ptr: Value::Var(n), ..
                }
                | Instruction::Store {
                    ptr: Value::Temp(n),
                    ..
                } => alloc_slot(n, false),
                Instruction::Unary { dst, op, value } => {
                    alloc_slot(dst, false);
                    if let Value::Var(n) | Value::Temp(n) = value {
                        alloc_slot(n, *op == IrOp::Ref);
                    }
                }
                Instruction::Binary { dst, lhs, rhs, .. } => {
                    alloc_slot(dst, false);
                    if let Value::Var(n) | Value::Temp(n) = lhs {
                        alloc_slot(n, false);
                    }
                    if let Value::Var(n) | Value::Temp(n) = rhs {
                        alloc_slot(n, false);
                    }
                }
                Instruction::Call {
                    dest: Some(dst), ..
                } => alloc_slot(dst, false),
                _ => {}
            }
        }

        let get_or_create_var_impl = |b: &mut FunctionBuilder,
                                      v_map: &mut HashMap<String, Variable>,
                                      v_idx: &mut usize,
                                      name: &str,
                                      ty: BackendType|
         -> Variable {
            *v_map.entry(name.to_string()).or_insert_with(|| {
                let v = Variable::new(*v_idx);
                *v_idx += 1;
                b.declare_var(v, ty.to_clif_type(ptr_type));
                v
            })
        };

        for inst in insts {
            if let Instruction::Label(lbl_name) = inst {
                let block = builder.create_block();
                all_blocks.push(block);
                label_map.insert(lbl_name.clone(), block);
            }
        }

        let mut call_args: Vec<cranelift::prelude::Value> = Vec::new();
        let mut call_arg_types: Vec<BackendType> = Vec::new();
        let mut param_counter = 0;

        for inst in insts {
            if let Some(current_block) = builder.current_block() {
                if let Some(last_inst) = builder.func.layout.last_inst(current_block) {
                    let opcode = builder.func.dfg.insts[last_inst].opcode();
                    if opcode.is_terminator()
                        && !matches!(inst, Instruction::FunctionLabel(_) | Instruction::Label(_))
                    {
                        continue;
                    }
                }
            }

            match inst {
                Instruction::Extern { .. } => {}
                Instruction::FunctionLabel(_) => {}

                Instruction::Param { p } => {
                    let cranelift_val = builder.block_params(entry_block)[param_counter];
                    param_counter += 1;

                    if let Some(frontend_type) = var_types.get(p) {
                        let abi =
                            AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                        if let AbiType::Aggregate { total_size, .. } = abi {
                            let slot = *stack_slot_map
                                .get(p)
                                .expect("Aggregate parameter missing frame");
                            let dst_addr = builder.ins().stack_addr(ptr_type, slot, 0);
                            let size_val = builder.ins().iconst(ptr_type, total_size as i64);
                            builder.call_memcpy(
                                self.module.target_config(),
                                dst_addr,
                                cranelift_val,
                                size_val,
                            );
                        } else {
                            let ty = BackendType::from_frontend(frontend_type);
                            let v = get_or_create_var_impl(
                                &mut builder,
                                &mut var_map,
                                &mut var_idx,
                                p,
                                ty,
                            );
                            builder.def_var(v, cranelift_val);
                        }
                    }
                }

                Instruction::Assign { dst, src } => {
                    let dst_frontend_ty = var_types.get(dst).cloned().unwrap_or(Type::Int);
                    let dst_ty = BackendType::from_frontend(&dst_frontend_ty);
                    let clif_ty = dst_ty.to_clif_type(ptr_type);

                    let abi = AbiType::from_frontend(&dst_frontend_ty, &self.struct_defs, ptr_type);
                    if let AbiType::Aggregate { total_size, .. } = abi {
                        let dst_slot = *stack_slot_map
                            .get(dst)
                            .expect("Target block space unallocated");
                        let dst_addr = builder.ins().stack_addr(ptr_type, dst_slot, 0);

                        match src {
                            Value::Var(name) | Value::Temp(name) => {
                                let src_slot = *stack_slot_map
                                    .get(name)
                                    .expect("Source block space unallocated");
                                let src_addr = builder.ins().stack_addr(ptr_type, src_slot, 0);
                                let size_val = builder.ins().iconst(ptr_type, total_size as i64);
                                builder.call_memcpy(
                                    self.module.target_config(),
                                    dst_addr,
                                    src_addr,
                                    size_val,
                                );
                            }
                            _ => {
                                if matches!(src, Value::Const(0)) {
                                    let size_val =
                                        builder.ins().iconst(ptr_type, total_size as i64);
                                    let zero =
                                        builder.ins().iconst(cranelift_codegen::ir::types::I8, 0);
                                    builder.call_memset(
                                        self.module.target_config(),
                                        dst_addr,
                                        zero,
                                        size_val,
                                    );
                                } else {
                                    panic!(
                                        "cannot assign scalar {:?} to aggregate destination",
                                        src
                                    );
                                }
                            }
                        }
                    } else {
                        let val = match src {
                            Value::Const(n) => builder.ins().iconst(clif_ty, *n),
                            Value::Bool(b) => builder.ins().iconst(clif_ty, if *b { 1 } else { 0 }),
                            Value::Var(name) | Value::Temp(name) => {
                                if let Some(slot) = stack_slot_map.get(name) {
                                    builder.ins().stack_load(clif_ty, *slot, 0)
                                } else {
                                    let src_ty = var_types
                                        .get(name)
                                        .map(BackendType::from_frontend)
                                        .unwrap_or(BackendType::Int64);
                                    let v = get_or_create_var_impl(
                                        &mut builder,
                                        &mut var_map,
                                        &mut var_idx,
                                        name,
                                        src_ty,
                                    );
                                    builder.use_var(v)
                                }
                            }
                            Value::Str(text) => {
                                let data_id = self.declare_string_literal(text);
                                let local_ref =
                                    self.module.declare_data_in_func(data_id, &mut builder.func);
                                builder.ins().global_value(clif_ty, local_ref)
                            }
                            Value::Char(ch) => {
                                let mut buffer = [0; 4];
                                let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                                builder.ins().iconst(clif_ty, byte_val as i64)
                            }
                            Value::Void => builder.ins().iconst(clif_ty, 0),
                        };

                        if let Some(slot) = stack_slot_map.get(dst) {
                            builder.ins().stack_store(val, *slot, 0);
                        } else {
                            let v = get_or_create_var_impl(
                                &mut builder,
                                &mut var_map,
                                &mut var_idx,
                                dst,
                                dst_ty,
                            );
                            builder.def_var(v, val);
                        }
                    }
                }

                Instruction::Binary { dst, op, lhs, rhs } => {
                    let dst_ty = var_types
                        .get(dst)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);
                    let clif_ty = dst_ty.to_clif_type(ptr_type);

                    let mut evaluate_operand =
                        |b: &mut FunctionBuilder, val: &Value, ty: BackendType| match val {
                            Value::Const(n) => b.ins().iconst(ty.to_clif_type(ptr_type), *n),
                            Value::Bool(bl) => b
                                .ins()
                                .iconst(ty.to_clif_type(ptr_type), if *bl { 1 } else { 0 }),
                            Value::Var(n) | Value::Temp(n) => {
                                if let Some(slot) = stack_slot_map.get(n) {
                                    b.ins().stack_load(ty.to_clif_type(ptr_type), *slot, 0)
                                } else {
                                    let src_ty = var_types
                                        .get(n)
                                        .map(BackendType::from_frontend)
                                        .unwrap_or(BackendType::Int64);
                                    let v = get_or_create_var_impl(
                                        b,
                                        &mut var_map,
                                        &mut var_idx,
                                        n,
                                        src_ty,
                                    );
                                    b.use_var(v)
                                }
                            }
                            _ => b.ins().iconst(ty.to_clif_type(ptr_type), 0),
                        };

                    let is_cmp = matches!(
                        op,
                        IrOp::Eq | IrOp::NEq | IrOp::Gt | IrOp::GtE | IrOp::Lt | IrOp::LtE
                    );

                    let operand_ty = if is_cmp {
                        match (lhs, rhs) {
                            (Value::Var(n) | Value::Temp(n), _) => var_types
                                .get(n)
                                .map(BackendType::from_frontend)
                                .unwrap_or(BackendType::Int64),
                            (_, Value::Var(n) | Value::Temp(n)) => var_types
                                .get(n)
                                .map(BackendType::from_frontend)
                                .unwrap_or(BackendType::Int64),
                            _ => BackendType::Int64,
                        }
                    } else {
                        dst_ty
                    };

                    let lhs_val = evaluate_operand(&mut builder, lhs, operand_ty);
                    let rhs_val = evaluate_operand(&mut builder, rhs, operand_ty);

                    let res = match op {
                        IrOp::Add => builder.ins().iadd(lhs_val, rhs_val),
                        IrOp::Sub => builder.ins().isub(lhs_val, rhs_val),
                        IrOp::Mul => builder.ins().imul(lhs_val, rhs_val),
                        IrOp::Div => builder.ins().sdiv(lhs_val, rhs_val),
                        IrOp::Mod => builder.ins().srem(lhs_val, rhs_val),
                        IrOp::Eq => builder.ins().icmp(IntCC::Equal, lhs_val, rhs_val),
                        IrOp::NEq => builder.ins().icmp(IntCC::NotEqual, lhs_val, rhs_val),
                        IrOp::Gt => builder
                            .ins()
                            .icmp(IntCC::SignedGreaterThan, lhs_val, rhs_val),
                        IrOp::GtE => {
                            builder
                                .ins()
                                .icmp(IntCC::SignedGreaterThanOrEqual, lhs_val, rhs_val)
                        }
                        IrOp::Lt => builder.ins().icmp(IntCC::SignedLessThan, lhs_val, rhs_val),
                        IrOp::LtE => {
                            builder
                                .ins()
                                .icmp(IntCC::SignedLessThanOrEqual, lhs_val, rhs_val)
                        }

                        IrOp::Pos | IrOp::Neg | IrOp::Ref => builder.ins().iconst(clif_ty, 0),
                    };

                    let res_clk_ty = builder.func.dfg.value_type(res);
                    let normalized_res = if is_cmp && res_clk_ty != clif_ty {
                        if res_clk_ty.bits() < clif_ty.bits() {
                            builder.ins().uextend(clif_ty, res)
                        } else if res_clk_ty.bits() > clif_ty.bits() {
                            builder.ins().ireduce(clif_ty, res)
                        } else {
                            res
                        }
                    } else {
                        res
                    };

                    if let Some(slot) = stack_slot_map.get(dst) {
                        builder.ins().stack_store(normalized_res, *slot, 0);
                    } else {
                        let v = get_or_create_var_impl(
                            &mut builder,
                            &mut var_map,
                            &mut var_idx,
                            dst,
                            dst_ty,
                        );
                        builder.def_var(v, normalized_res);
                    }
                }

                Instruction::Unary { dst, op, value } => {
                    let dst_ty = var_types
                        .get(dst)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);
                    let clif_ty = dst_ty.to_clif_type(ptr_type);

                    if *op == IrOp::Ref {
                        let addr_val = match value {
                            Value::Var(name) | Value::Temp(name) => {
                                let slot = *stack_slot_map
                                    .get(name)
                                    .expect("Target reference frame missing");
                                builder.ins().stack_addr(ptr_type, slot, 0)
                            }
                            literal => {
                                let lit_ty = get_val_backend_type(literal);
                                let clif_lit_ty = lit_ty.to_clif_type(ptr_type);

                                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                    StackSlotKind::ExplicitSlot,
                                    lit_ty.byte_size(),
                                ));

                                let val = match literal {
                                    Value::Char(ch) => {
                                        let mut buffer = [0; 4];
                                        let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                                        builder.ins().iconst(clif_lit_ty, byte_val as i64)
                                    }
                                    Value::Const(n) => builder.ins().iconst(clif_lit_ty, *n),
                                    Value::Bool(b) => {
                                        builder.ins().iconst(clif_lit_ty, if *b { 1 } else { 0 })
                                    }
                                    _ => builder.ins().iconst(clif_lit_ty, 0),
                                };

                                builder.ins().stack_store(val, slot, 0);
                                builder.ins().stack_addr(ptr_type, slot, 0)
                            }
                        };

                        if let Some(slot) = stack_slot_map.get(dst) {
                            builder.ins().stack_store(addr_val, *slot, 0);
                        } else {
                            let v = get_or_create_var_impl(
                                &mut builder,
                                &mut var_map,
                                &mut var_idx,
                                dst,
                                dst_ty,
                            );
                            builder.def_var(v, addr_val);
                        }
                    } else {
                        let inner_val = match value {
                            Value::Const(n) => builder.ins().iconst(clif_ty, *n),
                            Value::Var(n) | Value::Temp(n) => {
                                if let Some(slot) = stack_slot_map.get(n) {
                                    builder.ins().stack_load(clif_ty, *slot, 0)
                                } else {
                                    let src_ty = var_types
                                        .get(n)
                                        .map(BackendType::from_frontend)
                                        .unwrap_or(BackendType::Int64);
                                    let v = get_or_create_var_impl(
                                        &mut builder,
                                        &mut var_map,
                                        &mut var_idx,
                                        n,
                                        src_ty,
                                    );
                                    builder.use_var(v)
                                }
                            }
                            _ => builder.ins().iconst(clif_ty, 0),
                        };

                        let res = match op {
                            IrOp::Neg => builder.ins().ineg(inner_val),
                            IrOp::Pos => inner_val,
                            _ => inner_val,
                        };

                        if let Some(slot) = stack_slot_map.get(dst) {
                            builder.ins().stack_store(res, *slot, 0);
                        } else {
                            let v = get_or_create_var_impl(
                                &mut builder,
                                &mut var_map,
                                &mut var_idx,
                                dst,
                                dst_ty,
                            );
                            builder.def_var(v, res);
                        }
                    }
                }

                Instruction::Arg { value } => {
                    if let Value::Var(name) | Value::Temp(name) = value {
                        if let Some(frontend_type) = var_types.get(name) {
                            let abi =
                                AbiType::from_frontend(frontend_type, &self.struct_defs, ptr_type);
                            if let AbiType::Aggregate { .. } = abi {
                                let slot = *stack_slot_map.get(name).unwrap();
                                let addr_val = builder.ins().stack_addr(ptr_type, slot, 0);
                                call_args.push(addr_val);
                                call_arg_types.push(BackendType::Ptr);
                                continue;
                            }
                        }
                    }

                    let arg_ty = get_val_backend_type(value);
                    let mut val = match value {
                        Value::Const(n) => builder.ins().iconst(arg_ty.to_clif_type(ptr_type), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(arg_ty.to_clif_type(ptr_type), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder
                                    .ins()
                                    .stack_load(arg_ty.to_clif_type(ptr_type), *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    arg_ty,
                                );
                                builder.use_var(v)
                            }
                        }
                        Value::Void => builder.ins().iconst(arg_ty.to_clif_type(ptr_type), 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref =
                                self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder
                                .ins()
                                .global_value(arg_ty.to_clif_type(ptr_type), local_ref)
                        }
                        Value::Char(ch) => {
                            let mut buffer = [0; 4];
                            let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                            builder
                                .ins()
                                .iconst(arg_ty.to_clif_type(ptr_type), byte_val as i64)
                        }
                    };

                    if arg_ty.to_clif_type(ptr_type) == types::I8 {
                        val = builder.ins().uextend(types::I64, val);
                    }

                    call_args.push(val);
                    call_arg_types.push(if arg_ty.to_clif_type(ptr_type) == types::I8 {
                        BackendType::Int64
                    } else {
                        arg_ty
                    });
                }

                Instruction::Call {
                    dest,
                    name,
                    argc: _,
                } => {
                    let return_frontend_type = dest.as_ref().and_then(|d| var_types.get(d));
                    let return_type = return_frontend_type
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);

                    let is_extern = self.extern_names.contains(name);
                    let local_callee = self.get_or_declare_func(
                        name,
                        is_extern,
                        &call_arg_types,
                        return_frontend_type,
                    );
                    let local_clif_ref = self
                        .module
                        .declare_func_in_func(local_callee, &mut builder.func);

                    let call_inst = builder.ins().call(local_clif_ref, &call_args);
                    call_args.clear();
                    call_arg_types.clear();

                    if let Some(dst_str) = dest {
                        if dst_str != "n/a" {
                            let results = builder.inst_results(call_inst).to_vec();
                            if !results.is_empty() {
                                if let Some(front_ty) = return_frontend_type {
                                    let abi_type = AbiType::from_frontend(
                                        front_ty,
                                        &self.struct_defs,
                                        ptr_type,
                                    );
                                    match abi_type {
                                        AbiType::Primitive(_) => {
                                            if let Some(slot) = stack_slot_map.get(dst_str) {
                                                builder.ins().stack_store(results[0], *slot, 0);
                                            } else {
                                                let v = get_or_create_var_impl(
                                                    &mut builder,
                                                    &mut var_map,
                                                    &mut var_idx,
                                                    dst_str,
                                                    return_type,
                                                );
                                                builder.def_var(v, results[0]);
                                            }
                                        }
                                        AbiType::Aggregate { .. } => {
                                            let slot = *stack_slot_map
                                                .get(dst_str)
                                                .expect("Destination slot unallocated");
                                            let base_addr =
                                                builder.ins().stack_addr(ptr_type, slot, 0);

                                            for (i, &res_val) in results.iter().enumerate() {
                                                let offset_addr = builder
                                                    .ins()
                                                    .iadd_imm(base_addr, (i * 8) as i64);
                                                builder.ins().store(
                                                    MemFlags::new(),
                                                    res_val,
                                                    offset_addr,
                                                    0,
                                                );
                                            }
                                        }
                                        AbiType::Void => {}
                                    }
                                }
                            }
                        }
                    }
                }

                Instruction::Load {
                    dst,
                    ptr,
                    ty: frontend_load_ty,
                } => {
                    let ptr_ty = BackendType::Ptr;
                    let dst_ty = BackendType::from_frontend(frontend_load_ty);

                    let addr = match ptr {
                        Value::Const(n) => builder.ins().iconst(ptr_ty.to_clif_type(ptr_type), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(ptr_ty.to_clif_type(ptr_type), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_addr(ptr_type, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    ptr_ty,
                                );
                                builder.use_var(v)
                            }
                        }
                        Value::Void => builder.ins().iconst(ptr_ty.to_clif_type(ptr_type), 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref =
                                self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder
                                .ins()
                                .global_value(ptr_ty.to_clif_type(ptr_type), local_ref)
                        }
                        Value::Char(ch) => {
                            let mut buffer = [0; 4];
                            let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                            builder
                                .ins()
                                .iconst(ptr_ty.to_clif_type(ptr_type), byte_val as i64)
                        }
                    };

                    let loaded_val =
                        builder
                            .ins()
                            .load(dst_ty.to_clif_type(ptr_type), MemFlags::new(), addr, 0);
                    if let Some(slot) = stack_slot_map.get(dst) {
                        builder.ins().stack_store(loaded_val, *slot, 0);
                    } else {
                        let v = get_or_create_var_impl(
                            &mut builder,
                            &mut var_map,
                            &mut var_idx,
                            dst,
                            dst_ty,
                        );
                        builder.def_var(v, loaded_val);
                    }
                }

                Instruction::Store { ptr, source } => {
                    let ptr_ty = BackendType::Ptr;
                    let src_ty = get_val_backend_type(source);

                    let addr = match ptr {
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_addr(ptr_type, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    ptr_ty,
                                );
                                builder.use_var(v)
                            }
                        }
                        _ => panic!(
                            "Direct compilation targeting arbitrary absolute register addresses breaks bounds constraints"
                        ),
                    };

                    let src_val = match source {
                        Value::Const(n) => builder.ins().iconst(src_ty.to_clif_type(ptr_type), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(src_ty.to_clif_type(ptr_type), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder
                                    .ins()
                                    .stack_load(src_ty.to_clif_type(ptr_type), *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    src_ty,
                                );
                                builder.use_var(v)
                            }
                        }
                        Value::Char(ch) => {
                            let mut buffer = [0; 4];
                            let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                            builder
                                .ins()
                                .iconst(src_ty.to_clif_type(ptr_type), byte_val as i64)
                        }
                        _ => builder.ins().iconst(src_ty.to_clif_type(ptr_type), 0),
                    };

                    builder.ins().store(MemFlags::new(), src_val, addr, 0);
                }

                Instruction::Label(lbl_name) => {
                    let block = *label_map.get(lbl_name).unwrap();
                    if let Some(current_block) = builder.current_block() {
                        if builder
                            .func
                            .layout
                            .last_inst(current_block)
                            .map_or(true, |i| {
                                !builder.func.dfg.insts[i].opcode().is_terminator()
                            })
                        {
                            builder.ins().jump(block, &[]);
                        }
                    }
                    builder.switch_to_block(block);
                }

                Instruction::Jump(lbl_name) => {
                    let block = *label_map
                        .get(lbl_name)
                        .expect("Target basic block execution path segment missing");
                    builder.ins().jump(block, &[]);
                }

                Instruction::JumpIfFalse { cond, target } => {
                    let false_block = *label_map.get(target).expect("missing jump target");
                    let true_block = builder.create_block();
                    all_blocks.push(true_block);

                    let cond_val = match cond {
                        Value::Bool(v) => builder.ins().iconst(types::I8, if *v { 1 } else { 0 }),
                        Value::Const(v) => builder.ins().iconst(types::I64, *v),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(types::I8, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    BackendType::Bool,
                                );
                                builder.use_var(v)
                            }
                        }
                        _ => builder.ins().iconst(types::I8, 0),
                    };

                    builder
                        .ins()
                        .brif(cond_val, true_block, &[], false_block, &[]);
                    builder.switch_to_block(true_block);
                }

                Instruction::Return { value } => {
                    if let Some(front_ret) = current_func_ret_front {
                        let abi = AbiType::from_frontend(front_ret, &self.struct_defs, ptr_type);
                        if let AbiType::Aggregate { chunk_count, .. } = abi {
                            if let Value::Var(name) | Value::Temp(name) = value {
                                let slot = *stack_slot_map
                                    .get(name)
                                    .expect("Return aggregate slot missing");
                                let mut ret_vals = Vec::new();
                                for i in 0..chunk_count {
                                    let val =
                                        builder.ins().stack_load(types::I64, slot, (i * 8) as i32);
                                    ret_vals.push(val);
                                }
                                builder.ins().return_(&ret_vals);
                                continue;
                            }
                        }
                    }

                    let ret_ty = get_val_backend_type(value);
                    let val = match value {
                        Value::Const(n) => builder.ins().iconst(ret_ty.to_clif_type(ptr_type), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(ret_ty.to_clif_type(ptr_type), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder
                                    .ins()
                                    .stack_load(ret_ty.to_clif_type(ptr_type), *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    ret_ty,
                                );
                                builder.use_var(v)
                            }
                        }
                        _ => builder.ins().iconst(types::I64, 0),
                    };
                    builder.ins().return_(&[val]);
                }
            }
        }

        for &block in &all_blocks {
            if let Some(last_inst) = builder.func.layout.last_inst(block) {
                let opcode = builder.func.dfg.insts[last_inst].opcode();
                if !opcode.is_terminator() {
                    builder.switch_to_block(block);
                    let zero = builder.ins().iconst(types::I64, 0);
                    builder.ins().return_(&[zero]);
                }
            } else {
                builder.switch_to_block(block);
                let zero = builder.ins().iconst(types::I64, 0);
                builder.ins().return_(&[zero]);
            }
        }

        let mut sealed_blocks = std::collections::HashSet::new();
        sealed_blocks.insert(entry_block);

        for &block in &all_blocks {
            if sealed_blocks.insert(block) {
                builder.seal_block(block);
            }
        }

        builder.finalize();
        self.module.define_function(func_id, ctx).unwrap();
        self.module.clear_context(ctx);
    }
}
