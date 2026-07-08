use std::collections::HashMap;

use cranelift::{codegen::ir::StackSlot, prelude::*};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

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
    pub fn to_clif_type(&self) -> cranelift::prelude::Type {
        match self {
            BackendType::Char => types::I8,
            BackendType::Bool => types::I8,
            BackendType::Int32 => types::I32,
            BackendType::Int64 => types::I64,
            BackendType::Ptr => types::I64,
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

pub struct CraneliftBackend {
    module: ObjectModule,
    extern_names: std::collections::HashSet<String>,
    declared_funcs: HashMap<String, FuncId>,
    str_count: usize,
}

impl CraneliftBackend {
    pub fn new() -> Self {
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

    fn declare_string_literal(&mut self, text: &str) -> DataId {
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

        data_id
    }

    fn get_or_declare_func(
        &mut self,
        name: &str,
        public: bool,
        arg_types: &[BackendType],
        return_type: BackendType,
    ) -> FuncId {
        if let Some(&id) = self.declared_funcs.get(name) {
            return id;
        }

        let mut callee_sig = self.module.make_signature();
        callee_sig
            .returns
            .push(AbiParam::new(return_type.to_clif_type()));
        for ty in arg_types {
            callee_sig.params.push(AbiParam::new(ty.to_clif_type()));
        }

        let linkage = if self.extern_names.contains(name) {
            Linkage::Import
        } else if public {
            Linkage::Export
        } else {
            Linkage::Local
        };

        let id = self
            .module
            .declare_function(name, linkage, &callee_sig)
            .unwrap();
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

        sig.returns.push(AbiParam::new(types::I64));

        for inst in insts {
            if let Instruction::Param { p } = inst {
                let ty = var_types
                    .get(p)
                    .map(BackendType::from_frontend)
                    .unwrap_or(BackendType::Int64);
                sig.params.push(AbiParam::new(ty.to_clif_type()));
            }
        }

        let func_id = self.get_or_declare_func(name, public, &[], BackendType::Int64);
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
            if let Instruction::Unary {
                op: IrOp::Ref,
                value: Value::Var(name) | Value::Temp(name),
                ..
            } = inst
            {
                let frontend_type = var_types.get(name).cloned().unwrap_or(Type::Int);
                let total_size = match frontend_type {
                    Type::Array {
                        ref element_type,
                        size,
                    } => {
                        let elem_size = BackendType::from_frontend(element_type).byte_size();
                        elem_size * (size as u32)
                    }
                    other => BackendType::from_frontend(&other).byte_size(),
                };

                stack_slot_map.entry(name.clone()).or_insert_with(|| {
                    builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        total_size,
                    ))
                });
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
                b.declare_var(v, ty.to_clif_type());
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
                Instruction::Param { p } => {
                    let cranelift_val = builder.block_params(entry_block)[param_counter];
                    param_counter += 1;
                    let ty = var_types
                        .get(p)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);
                    let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, p, ty);
                    builder.def_var(v, cranelift_val);
                }
                Instruction::Assign { dst, src } => {
                    let dst_ty = var_types
                        .get(dst)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);
                    let clif_ty = dst_ty.to_clif_type();

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
                Instruction::Binary { dst, op, lhs, rhs } => {
                    let dst_ty = var_types
                        .get(dst)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);

                    let l_ty = get_val_backend_type(lhs);
                    let r_ty = get_val_backend_type(rhs);

                    let l = match lhs {
                        Value::Const(n) => builder.ins().iconst(l_ty.to_clif_type(), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(l_ty.to_clif_type(), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(l_ty.to_clif_type(), *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    l_ty,
                                );
                                builder.use_var(v)
                            }
                        }
                        Value::Void => builder.ins().iconst(l_ty.to_clif_type(), 0),
                        Value::Str(_) => unreachable!(),
                        Value::Char(_) => unreachable!(),
                    };

                    let r = match rhs {
                        Value::Const(n) => builder.ins().iconst(r_ty.to_clif_type(), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(r_ty.to_clif_type(), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(r_ty.to_clif_type(), *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    r_ty,
                                );
                                builder.use_var(v)
                            }
                        }
                        Value::Void => builder.ins().iconst(r_ty.to_clif_type(), 0),
                        Value::Str(_) => unreachable!(),
                        Value::Char(_) => unreachable!(),
                    };

                    let zero = builder.ins().iconst(dst_ty.to_clif_type(), 0);
                    let one = builder.ins().iconst(dst_ty.to_clif_type(), 1);

                    let res = match op {
                        IrOp::Add => builder.ins().iadd(l, r),
                        IrOp::Sub => builder.ins().isub(l, r),
                        IrOp::Mul => builder.ins().imul(l, r),
                        IrOp::Div => builder.ins().sdiv(l, r),
                        IrOp::Mod => builder.ins().srem(l, r),

                        IrOp::Eq => {
                            let c = builder.ins().icmp(IntCC::Equal, l, r);
                            builder.ins().select(c, one, zero)
                        }
                        IrOp::NEq => {
                            let c = builder.ins().icmp(IntCC::NotEqual, l, r);
                            builder.ins().select(c, one, zero)
                        }
                        IrOp::Lt => {
                            let c = builder.ins().icmp(IntCC::SignedLessThan, l, r);
                            builder.ins().select(c, one, zero)
                        }
                        IrOp::LtE => {
                            let c = builder.ins().icmp(IntCC::SignedLessThanOrEqual, l, r);
                            builder.ins().select(c, one, zero)
                        }
                        IrOp::Gt => {
                            let c = builder.ins().icmp(IntCC::SignedGreaterThan, l, r);
                            builder.ins().select(c, one, zero)
                        }
                        IrOp::GtE => {
                            let c = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, l, r);
                            builder.ins().select(c, one, zero)
                        }
                        _ => panic!("Expected binary operator, found unary: {:?}", op),
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
                Instruction::Unary { dst, op, value } => {
                    let dst_ty = var_types
                        .get(dst)
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);
                    let val_ty = get_val_backend_type(value);

                    let val = match value {
                        Value::Const(n) => builder.ins().iconst(val_ty.to_clif_type(), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(val_ty.to_clif_type(), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                if matches!(op, IrOp::Ref) {
                                    builder.ins().stack_addr(types::I64, *slot, 0)
                                } else {
                                    builder.ins().stack_load(val_ty.to_clif_type(), *slot, 0)
                                }
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    val_ty,
                                );
                                builder.use_var(v)
                            }
                        }
                        Value::Void => builder.ins().iconst(val_ty.to_clif_type(), 0),
                        Value::Str(_) => unreachable!(),
                        Value::Char(_) => unreachable!(),
                    };

                    let res = match op {
                        IrOp::Neg => builder.ins().ineg(val),
                        IrOp::Pos | IrOp::Ref => val,
                        _ => panic!("Expected unary operator, found binary: {:?}", op),
                    };

                    let v = get_or_create_var_impl(
                        &mut builder,
                        &mut var_map,
                        &mut var_idx,
                        dst,
                        dst_ty,
                    );
                    builder.def_var(v, res);
                }
                Instruction::Label(lbl_name) => {
                    let target_block = *label_map.get(lbl_name).unwrap();
                    if let Some(current_block) = builder.current_block() {
                        if let Some(last_inst) = builder.func.layout.last_inst(current_block) {
                            let opcode = builder.func.dfg.insts[last_inst].opcode();
                            if !opcode.is_terminator() {
                                builder.ins().jump(target_block, &[]);
                            }
                        } else {
                            builder.ins().jump(target_block, &[]);
                        }
                    }
                    builder.switch_to_block(target_block);
                }
                Instruction::Jump(lbl_name) => {
                    let target_block = *label_map.get(lbl_name).unwrap();
                    builder.ins().jump(target_block, &[]);
                }
                Instruction::JumpIfFalse { cond, target } => {
                    let cond_ty = get_val_backend_type(cond);
                    let c_val = match cond {
                        Value::Const(n) => builder.ins().iconst(cond_ty.to_clif_type(), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(cond_ty.to_clif_type(), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(cond_ty.to_clif_type(), *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(
                                    &mut builder,
                                    &mut var_map,
                                    &mut var_idx,
                                    name,
                                    cond_ty,
                                );
                                builder.use_var(v)
                            }
                        }
                        Value::Void => builder.ins().iconst(cond_ty.to_clif_type(), 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref =
                                self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder
                                .ins()
                                .global_value(cond_ty.to_clif_type(), local_ref)
                        }
                        Value::Char(ch) => {
                            let mut buffer = [0; 4];
                            let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                            builder
                                .ins()
                                .iconst(cond_ty.to_clif_type(), byte_val as i64)
                        }
                    };

                    let target_block = *label_map.get(target).unwrap();
                    let next_block = builder.create_block();
                    all_blocks.push(next_block);
                    let is_zero = builder.ins().icmp_imm(IntCC::Equal, c_val, 0);

                    builder
                        .ins()
                        .brif(is_zero, target_block, &[], next_block, &[]);
                    builder.switch_to_block(next_block);
                }
                Instruction::Arg { value } => {
                    let arg_ty = get_val_backend_type(value);
                    let mut val = match value {
                        Value::Const(n) => builder.ins().iconst(arg_ty.to_clif_type(), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(arg_ty.to_clif_type(), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(arg_ty.to_clif_type(), *slot, 0)
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
                        Value::Void => builder.ins().iconst(arg_ty.to_clif_type(), 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref =
                                self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(arg_ty.to_clif_type(), local_ref)
                        }
                        Value::Char(ch) => {
                            let mut buffer = [0; 4];
                            let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                            builder.ins().iconst(arg_ty.to_clif_type(), byte_val as i64)
                        }
                    };

                    let final_ty = if arg_ty.to_clif_type() == types::I8 {
                        val = builder.ins().uextend(types::I64, val);
                        BackendType::Int64
                    } else {
                        arg_ty
                    };

                    call_args.push(val);
                    call_arg_types.push(final_ty);
                }
                Instruction::Call {
                    dest,
                    name,
                    argc: _,
                } => {
                    let return_type = dest
                        .as_ref()
                        .and_then(|d| var_types.get(d))
                        .map(BackendType::from_frontend)
                        .unwrap_or(BackendType::Int64);

                    let is_extern = self.extern_names.contains(name);

                    let local_callee =
                        self.get_or_declare_func(name, is_extern, &call_arg_types, return_type);
                    let local_clif_ref = self
                        .module
                        .declare_func_in_func(local_callee, &mut builder.func);

                    let call_inst = builder.ins().call(local_clif_ref, &call_args);
                    call_args.clear();
                    call_arg_types.clear();

                    if let Some(dst_str) = dest {
                        let res_val = builder.inst_results(call_inst)[0];
                        if let Some(slot) = stack_slot_map.get(dst_str) {
                            builder.ins().stack_store(res_val, *slot, 0);
                        } else {
                            let v = get_or_create_var_impl(
                                &mut builder,
                                &mut var_map,
                                &mut var_idx,
                                dst_str,
                                return_type,
                            );
                            builder.def_var(v, res_val);
                        }
                    }
                }
                Instruction::Return { value } => {
                    let ret_ty = get_val_backend_type(value);
                    let ret_val = match value {
                        Value::Const(n) => builder.ins().iconst(ret_ty.to_clif_type(), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(ret_ty.to_clif_type(), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(ret_ty.to_clif_type(), *slot, 0)
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
                        Value::Void => builder.ins().iconst(ret_ty.to_clif_type(), 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref =
                                self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(ret_ty.to_clif_type(), local_ref)
                        }
                        Value::Char(ch) => {
                            let mut buffer = [0; 4];
                            let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                            builder.ins().iconst(ret_ty.to_clif_type(), byte_val as i64)
                        }
                    };
                    builder.ins().return_(&[ret_val]);
                }
                Instruction::Store { ptr, source } => {
                    let ptr_ty = BackendType::Ptr;
                    let src_ty = get_val_backend_type(source);

                    let addr = match ptr {
                        Value::Const(n) => builder.ins().iconst(ptr_ty.to_clif_type(), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(ptr_ty.to_clif_type(), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(ptr_ty.to_clif_type(), *slot, 0)
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
                        Value::Void => builder.ins().iconst(ptr_ty.to_clif_type(), 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref =
                                self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(ptr_ty.to_clif_type(), local_ref)
                        }
                        Value::Char(ch) => {
                            let mut buffer = [0; 4];
                            let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                            builder.ins().iconst(ptr_ty.to_clif_type(), byte_val as i64)
                        }
                    };

                    let val = match source {
                        Value::Const(n) => builder.ins().iconst(src_ty.to_clif_type(), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(src_ty.to_clif_type(), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(src_ty.to_clif_type(), *slot, 0)
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
                        Value::Void => builder.ins().iconst(src_ty.to_clif_type(), 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref =
                                self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(src_ty.to_clif_type(), local_ref)
                        }
                        Value::Char(ch) => {
                            let mut buffer = [0; 4];
                            let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                            builder.ins().iconst(src_ty.to_clif_type(), byte_val as i64)
                        }
                    };
                    builder.ins().store(MemFlags::new(), val, addr, 0);
                }
                Instruction::Load {
                    dst,
                    ptr,
                    ty: frontend_load_ty,
                } => {
                    let ptr_ty = BackendType::Ptr;
                    let dst_ty = BackendType::from_frontend(frontend_load_ty);

                    let addr = match ptr {
                        Value::Const(n) => builder.ins().iconst(ptr_ty.to_clif_type(), *n),
                        Value::Bool(b) => builder
                            .ins()
                            .iconst(ptr_ty.to_clif_type(), if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_addr(types::I64, *slot, 0)
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
                        Value::Void => builder.ins().iconst(ptr_ty.to_clif_type(), 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref =
                                self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(ptr_ty.to_clif_type(), local_ref)
                        }
                        Value::Char(ch) => {
                            let mut buffer = [0; 4];
                            let byte_val = ch.encode_utf8(&mut buffer).as_bytes()[0];
                            builder.ins().iconst(ptr_ty.to_clif_type(), byte_val as i64)
                        }
                    };

                    let loaded_val =
                        builder
                            .ins()
                            .load(dst_ty.to_clif_type(), MemFlags::new(), addr, 0);

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
                _ => {}
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
