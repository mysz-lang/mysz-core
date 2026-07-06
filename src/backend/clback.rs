use std::collections::HashMap;

use cranelift::{codegen::ir::StackSlot, prelude::*};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::ir::tac::{Instruction, IrOp, Value};

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
        let isa = isa_builder.finish(settings::Flags::new(flag_builder)).unwrap();

        let builder = ObjectBuilder::new(
            isa,
            "mysz_prog",
            cranelift_module::default_libcall_names()
        ).unwrap();

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

        let data_id = self.module.declare_data(&sym_name, Linkage::Local, false, false).unwrap();
        self.module.define_data(data_id, &data_desc).unwrap();

        data_id
    }

    /// Look up (declaring if needed) the FuncId for a call target, using
    /// Import linkage for externs and Local linkage for functions defined
    /// elsewhere in this module. Cached so repeated calls to the same
    /// function don't re-declare it.
    fn get_or_declare_func(&mut self, name: &str, argc: usize) -> FuncId {
        if let Some(&id) = self.declared_funcs.get(name) {
            return id;
        }

        let mut callee_sig = self.module.make_signature();
        callee_sig.returns.push(AbiParam::new(types::I64));
        for _ in 0..argc {
            callee_sig.params.push(AbiParam::new(types::I64));
        }

        let linkage = if self.extern_names.contains(name) {
            Linkage::Import
        } else {
            Linkage::Local
        };

        let id = self.module.declare_function(name, linkage, &callee_sig).unwrap();
        self.declared_funcs.insert(name.to_string(), id);
        id
    }

    pub fn compile_function(
        &mut self,
        name: &str,
        insts: &[&Instruction],
        ctx: &mut cranelift::codegen::Context,
        func_ctx: &mut FunctionBuilderContext
    ) {
        let mut sig = self.module.make_signature();
        sig.returns.push(AbiParam::new(types::I64));

        for inst in insts {
            if let Instruction::Param { .. } = inst {
                sig.params.push(AbiParam::new(types::I64));
            }
        }

        // This function is defined in this module -- Export makes it visible
        // to the linker (e.g. so `main` is callable from the C runtime startup).
        let func_id = self.module.declare_function(name, Linkage::Export, &sig).unwrap();
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

        // Pre-allocate stack slots for referenced variables/temps safely
        for inst in insts {
            match inst {
                Instruction::Unary { op: IrOp::Ref, value: Value::Var(name) | Value::Temp(name), .. } => {
                    stack_slot_map.entry(name.clone()).or_insert_with(|| {
                        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 24))
                    });
                }
                _ => {}
            }
        }

        let get_or_create_var_impl = |b: &mut FunctionBuilder, v_map: &mut HashMap<String, Variable>, v_idx: &mut usize, name: &str| -> Variable {
            *v_map.entry(name.to_string()).or_insert_with(|| {
                let v = Variable::new(*v_idx);
                *v_idx += 1;
                b.declare_var(v, types::I64);
                v
            })
        };

        // Pre-declare your cross-jump labels
        for inst in insts {
            if let Instruction::Label(lbl_name) = inst {
                let block = builder.create_block();
                all_blocks.push(block);
                label_map.insert(lbl_name.clone(), block);
            }
        }

        let mut call_args: Vec<cranelift::prelude::Value> = Vec::new();
        let mut param_counter = 0;

        for inst in insts {
            if let Some(current_block) = builder.current_block() {
                if let Some(last_inst) = builder.func.layout.last_inst(current_block) {
                    let opcode = builder.func.dfg.insts[last_inst].opcode();
                    if opcode.is_terminator() {
                        if matches!(inst, Instruction::FunctionLabel(_) | Instruction::Label(_)) {
                        } else {
                            continue;
                        }
                    }
                }
            }

            match inst {
                Instruction::Param { p } => {
                    let cranelift_val = builder.block_params(entry_block)[param_counter];
                    param_counter += 1;
                    let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, p);
                    builder.def_var(v, cranelift_val);
                }
                Instruction::Assign { dst, src } => {
                    let val = match src {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(types::I64, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref = self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(types::I64, local_ref)
                        }
                        Value::Void => builder.ins().iconst(types::I64, 0),
                    };

                    if let Some(slot) = stack_slot_map.get(dst) {
                        builder.ins().stack_store(val, *slot, 0);
                    } else {
                        let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, dst);
                        builder.def_var(v, val);
                    }
                }
                Instruction::Binary { dst, op, lhs, rhs } => {
                    let l = match lhs {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(types::I64, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Void => builder.ins().iconst(types::I64, 0),
                        Value::Str(_) => unreachable!(),
                    };
                    let r = match rhs {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(types::I64, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Void => builder.ins().iconst(types::I64, 0),
                        Value::Str(_) => unreachable!(),
                    };

                    let res = match op {
                        IrOp::Add => builder.ins().iadd(l, r),
                        IrOp::Sub => builder.ins().isub(l, r),
                        IrOp::Mul => builder.ins().imul(l, r),
                        IrOp::Div => builder.ins().sdiv(l, r),

                        IrOp::Eq  => { let c = builder.ins().icmp(IntCC::Equal, l, r); builder.ins().uextend(types::I64, c) },
                        IrOp::NEq => { let c = builder.ins().icmp(IntCC::NotEqual, l, r); builder.ins().uextend(types::I64, c) },
                        IrOp::Lt  => { let c = builder.ins().icmp(IntCC::SignedLessThan, l, r); builder.ins().uextend(types::I64, c) },
                        IrOp::LtE => { let c = builder.ins().icmp(IntCC::SignedLessThanOrEqual, l, r); builder.ins().uextend(types::I64, c) },
                        IrOp::Gt  => { let c = builder.ins().icmp(IntCC::SignedGreaterThan, l, r); builder.ins().uextend(types::I64, c) },
                        IrOp::GtE => { let c = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, l, r); builder.ins().uextend(types::I64, c) },

                        IrOp::Mod => builder.ins().srem(l, r),

                        _ => panic!("Expected binary operator, found unary: {:?}", op),
                    };

                    if let Some(slot) = stack_slot_map.get(dst) {
                        builder.ins().stack_store(res, *slot, 0);
                    } else {
                        let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, dst);
                        builder.def_var(v, res);
                    }
                }
                Instruction::Unary { dst, op, value } => {
                    let val = match value {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                if matches!(op, IrOp::Ref) {
                                    builder.ins().stack_addr(types::I64, *slot, 0)
                                } else {
                                    builder.ins().stack_load(types::I64, *slot, 0)
                                }
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Void => builder.ins().iconst(types::I64, 0),
                        Value::Str(_) => unreachable!(),
                    };

                    let res = match op {
                        IrOp::Neg => builder.ins().ineg(val),
                        IrOp::Pos => val,
                        IrOp::Ref => val,
                        _ => panic!("Expected unary operator, found binary: {:?}", op),
                    };

                    let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, dst);
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
                    let c_val = match cond {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(types::I64, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Void => builder.ins().iconst(types::I64, 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref = self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(types::I64, local_ref)
                        }
                    };

                    let target_block = *label_map.get(target).unwrap();
                    let next_block = builder.create_block();
                    all_blocks.push(next_block);
                    let is_zero = builder.ins().icmp_imm(IntCC::Equal, c_val, 0);

                    builder.ins().brif(is_zero, target_block, &[], next_block, &[]);

                    builder.switch_to_block(next_block);
                }
                Instruction::Arg { value } => {
                    let val = match value {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(types::I64, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Void => builder.ins().iconst(types::I64, 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref = self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(types::I64, local_ref)
                        }
                    };
                    call_args.push(val);
                }
                Instruction::Call { dest, name, argc } => {
                    let local_callee = self.get_or_declare_func(name, *argc);
                    let local_clif_ref = self.module.declare_func_in_func(local_callee, &mut builder.func);

                    let call_inst = builder.ins().call(local_clif_ref, &call_args);
                    call_args.clear();

                    if let Some(dst_str) = dest {
                        let res_val = builder.inst_results(call_inst)[0];
                        if let Some(slot) = stack_slot_map.get(dst_str) {
                            builder.ins().stack_store(res_val, *slot, 0);
                        } else {
                            let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, dst_str);
                            builder.def_var(v, res_val);
                        }
                    }
                }
                Instruction::Return { value } => {
                    let ret_val = match value {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(types::I64, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Void => builder.ins().iconst(types::I64, 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref = self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(types::I64, local_ref)
                        }
                    };
                    builder.ins().return_(&[ret_val]);
                }
                Instruction::Store { ptr, source } => {
                    let addr = match ptr {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(types::I64, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Void => builder.ins().iconst(types::I64, 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref = self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(types::I64, local_ref)
                        }
                    };
                    let val = match source {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_load(types::I64, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Void => builder.ins().iconst(types::I64, 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref = self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(types::I64, local_ref)
                        }
                    };
                    builder.ins().store(MemFlags::new(), val, addr, 0);
                }
                Instruction::Load { dst, ptr, .. } => {
                    let addr = match ptr {
                        Value::Const(n) => builder.ins().iconst(types::I64, *n),
                        Value::Bool(b) => builder.ins().iconst(types::I64, if *b { 1 } else { 0 }),
                        Value::Var(name) | Value::Temp(name) => {
                            if let Some(slot) = stack_slot_map.get(name) {
                                builder.ins().stack_addr(types::I64, *slot, 0)
                            } else {
                                let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, name);
                                builder.use_var(v)
                            }
                        },
                        Value::Void => builder.ins().iconst(types::I64, 0),
                        Value::Str(text) => {
                            let data_id = self.declare_string_literal(text);
                            let local_ref = self.module.declare_data_in_func(data_id, &mut builder.func);
                            builder.ins().global_value(types::I64, local_ref)
                        }
                    };
                    let loaded_val = builder.ins().load(types::I64, MemFlags::new(), addr, 0);

                    if let Some(slot) = stack_slot_map.get(dst) {
                        builder.ins().stack_store(loaded_val, *slot, 0);
                    } else {
                        let v = get_or_create_var_impl(&mut builder, &mut var_map, &mut var_idx, dst);
                        builder.def_var(v, loaded_val);
                    }
                }
                _ => {}
            }
        }
            for &block in &all_blocks {
            if let Some(_) = builder.func.layout.first_inst(block) {
                // The block has instructions, so check the last one for a terminator
                if let Some(last_inst) = builder.func.layout.last_inst(block) {
                    let opcode = builder.func.dfg.insts[last_inst].opcode();
                    if !opcode.is_terminator() {
                        builder.switch_to_block(block);
                        let zero = builder.ins().iconst(types::I64, 0);
                        builder.ins().return_(&[zero]);
                    }
                }
            } else {
                // Fully empty basic block fallback: add a fallback terminator
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