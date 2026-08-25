use std::collections::HashMap;

use inkwell::{
    IntPredicate,
    basic_block::BasicBlock,
    builder::Builder,
    context::Context,
    module::Module,
    types::{BasicType, BasicTypeEnum, StructType},
    values::{BasicValueEnum, FunctionValue, GlobalValue, IntValue, PointerValue},
};

use crate::{
    ir::{
        irgen::StructLayout,
        tac::{CastType, Instruction, IrOp, ScopedMap, Value},
    },
    parse::parsing::Type,
    semantics::analysis::FunctionSignature,
    utils::typesafe::types_equal,
};

use crate::utils::typesafe::{is_integer, is_signed_integer, is_truthy_type, type_to_string};

pub struct LlvmBackend<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,

    functions: HashMap<String, FunctionValue<'ctx>>,
    blocks: HashMap<String, BasicBlock<'ctx>>,
    pending_args: Vec<BasicValueEnum<'ctx>>,
    current_param_idx: u32,

    temps: HashMap<String, BasicValueEnum<'ctx>>,
    temp_types: HashMap<String, Type>,
    vars: HashMap<String, PointerValue<'ctx>>,
    strings: HashMap<String, GlobalValue<'ctx>>,
    struct_types: HashMap<String, StructType<'ctx>>,

    var_types: ScopedMap,
    struct_defs: HashMap<String, StructLayout>,
    func_defs: HashMap<String, FunctionSignature>,

    next_block_id: usize,
}

/// Helpers
impl<'ctx> LlvmBackend<'ctx> {
    pub fn new(
        context: &'ctx Context,
        module_name: &str,
        var_types: ScopedMap,
        struct_defs: HashMap<String, StructLayout>,
        func_defs: HashMap<String, FunctionSignature>,
    ) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();

        Self {
            context,
            module,
            builder,

            functions: HashMap::new(),
            blocks: HashMap::new(),
            pending_args: Vec::new(),
            current_param_idx: 0,

            temps: HashMap::new(),
            temp_types: HashMap::new(),
            vars: HashMap::new(),
            strings: HashMap::new(),
            struct_types: HashMap::new(),

            var_types,
            struct_defs,
            func_defs,

            next_block_id: 0,
        }
    }

    pub fn module(&self) -> &Module<'ctx> {
        &self.module
    }

    fn fresh_block_name(&mut self, prefix: &str) -> String {
        let id = self.next_block_id;
        self.next_block_id += 1;

        format!("{}.{}", prefix, id)
    }

    pub fn verify(&self) -> Result<(), String> {
        self.module.verify().map_err(|err| err.to_string())
    }

    pub fn print_ir(&self) {
        self.module.print_to_stderr();
    }

    pub fn compile(&mut self, instructions: &[Instruction]) -> Result<(), String> {
        self.declare_structs()?;
        self.declare_functions(instructions)?;
        self.create_blocks(instructions)?;

        self.compile_instructions(instructions)?;

        Ok(())
    }

    fn compile_instructions(&mut self, instructions: &[Instruction]) -> Result<(), String> {
        for instruction in instructions.iter() {
            self.compile_instruction(instruction)?;
        }

        Ok(())
    }

    fn temp_type(&self, name: &str) -> Result<&Type, String> {
        self.temp_types
            .get(name)
            .ok_or_else(|| format!("unknown temporary '{}'", name))
    }

    fn value_type(&self, value: &Value) -> Result<Type, String> {
        match value {
            Value::Const(_) => Ok(Type::Int),
            Value::Bool(_) => Ok(Type::Bool),
            Value::Char(_) => Ok(Type::Char),

            Value::Temp(name) => Ok(self.temp_type(name)?.clone()),

            Value::Var(name) => self
                .var_types
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown variable '{}'", name)),

            Value::Void => Ok(Type::Void),

            Value::Str(_) => Ok(Type::Str),
        }
    }

    fn llvm_type(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        match ty {
            Type::Int | Type::UInt => self.context.i64_type().into(),

            Type::Int8 | Type::UInt8 | Type::Char => self.context.i8_type().into(),

            Type::Bool => self.context.bool_type().into(),

            Type::Ptr(_) => self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into(),

            Type::Array { element_type, size } => self.llvm_array_type(element_type, *size),

            Type::Str => self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into(),

            Type::Struct(name) => self
                .struct_types
                .get(name)
                .copied()
                .unwrap_or_else(|| panic!("unknown struct '{}'", name))
                .into(),

            Type::Void => {
                panic!("ICE: void used where an LLVM value type was required")
            }

            Type::GenericInstance { .. }
            | Type::GenericParam(_)
            | Type::VariadicPack { .. }
            | Type::Any => {
                panic!("ICE: frontend-only type reached LLVM backend")
            }
        }
    }

    fn llvm_value(&mut self, value: &Value) -> Result<BasicValueEnum<'ctx>, String> {
        let ty = self.value_type(value)?;

        match value {
            Value::Const(value) => match ty {
                Type::Int => Ok(self
                    .context
                    .i64_type()
                    .const_int(*value as u64, true)
                    .into()),

                Type::UInt => Ok(self
                    .context
                    .i64_type()
                    .const_int(*value as u64, false)
                    .into()),

                Type::Int8 => Ok(self.context.i8_type().const_int(*value as u64, true).into()),

                Type::UInt8 | Type::Char => Ok(self
                    .context
                    .i8_type()
                    .const_int(*value as u64, false)
                    .into()),

                _ => Err("invalid type for integer constant".to_string()),
            },

            Value::Bool(value) => Ok(self
                .context
                .bool_type()
                .const_int(*value as u64, false)
                .into()),

            Value::Char(value) => Ok(self
                .context
                .i8_type()
                .const_int(*value as u8 as u64, false)
                .into()),

            Value::Temp(name) => self
                .temps
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown temporary '{}'", name)),

            Value::Var(name) => {
                let ptr = self
                    .vars
                    .get(name)
                    .copied()
                    .ok_or_else(|| format!("unknown variable '{}'", name))?;

                let llvm_ty = self.llvm_type(&ty);

                self.builder
                    .build_load(llvm_ty, ptr, name)
                    .map_err(|err| err.to_string())
            }

            Value::Str(value) => Ok(self.llvm_string(value)?.into()),

            Value::Void => Err("void is not an LLVM value".to_string()),
        }
    }

    fn llvm_string(&mut self, value: &str) -> Result<PointerValue<'ctx>, String> {
        if let Some(global) = self.strings.get(value).copied() {
            return Ok(global.as_pointer_value());
        }

        let name = format!("str.{}", self.strings.len());

        let global = self
            .builder
            .build_global_string_ptr(value, &name)
            .map_err(|err| err.to_string())?;

        global.set_linkage(inkwell::module::Linkage::Private);
        global.set_constant(true);

        let ptr = global.as_pointer_value();

        self.strings.insert(value.to_string(), global);

        Ok(ptr)
    }

    fn llvm_array_type(&self, element_type: &Type, size: usize) -> BasicTypeEnum<'ctx> {
        match self.llvm_type(element_type) {
            BasicTypeEnum::ArrayType(ty) => ty.array_type(size as u32).into(),

            BasicTypeEnum::IntType(ty) => ty.array_type(size as u32).into(),

            BasicTypeEnum::FloatType(ty) => ty.array_type(size as u32).into(),

            BasicTypeEnum::PointerType(ty) => ty.array_type(size as u32).into(),

            BasicTypeEnum::StructType(ty) => ty.array_type(size as u32).into(),

            BasicTypeEnum::VectorType(ty) => ty.array_type(size as u32).into(),

            BasicTypeEnum::ScalableVectorType(ty) => ty.array_type(size as u32).into(),
        }
    }

    fn llvm_ptr(&self, value: &Value) -> Result<PointerValue<'ctx>, String> {
        match value {
            Value::Var(name) => self
                .vars
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown variable '{}'", name)),

            Value::Temp(name) => {
                let value = self
                    .temps
                    .get(name)
                    .copied()
                    .ok_or_else(|| format!("unknown temporary '{}'", name))?;

                if !value.is_pointer_value() {
                    return Err(format!("temporary '{}' is not a pointer", name));
                }

                Ok(value.into_pointer_value())
            }

            _ => Err("cannot use this value as a pointer".to_string()),
        }
    }

    fn llvm_truthy(&mut self, value: &Value) -> Result<IntValue<'ctx>, String> {
        let ty = self.value_type(value)?;

        if !is_truthy_type(&ty) {
            return Err(format!(
                "ICE: type {} cannot be used as a condition",
                type_to_string(&ty)
            ));
        }

        let value = self.llvm_value(value)?;

        match ty {
            Type::Bool => Ok(value.into_int_value()),

            Type::Int | Type::UInt | Type::Int8 | Type::UInt8 => {
                let value = value.into_int_value();
                let zero = value.get_type().const_zero();

                self.builder
                    .build_int_compare(IntPredicate::NE, value, zero, "truthy")
                    .map_err(|err| err.to_string())
            }

            Type::Str => {
                let ptr = value.into_pointer_value();
                let null = ptr.get_type().const_null();

                self.builder
                    .build_int_compare(IntPredicate::NE, ptr, null, "str.truthy")
                    .map_err(|err| err.to_string())
            }

            _ => {
                // is_truthy_type() guarantees this should be unreachable.
                panic!(
                    "ICE: truthy type {} has no LLVM truthiness implementation",
                    type_to_string(&ty)
                );
            }
        }
    }

    fn build_int_compare(
        &self,
        predicate: IntPredicate,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_int_compare(predicate, lhs, rhs, name)
            .map(|value| value.into())
            .map_err(|err| err.to_string())
    }

    fn current_function_name(&self) -> Result<String, String> {
        self.builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .map(|function| function.get_name().to_string_lossy().into_owned())
            .ok_or_else(|| "no current function".to_string())
    }

    fn get_block(&self, label: &str) -> Result<BasicBlock<'ctx>, String> {
        let function_name = self.current_function_name()?;

        self.blocks
            .get(&format!("{}::{}", function_name, label))
            .copied()
            .ok_or_else(|| format!("unknown label '{}' in function '{}'", label, function_name))
    }

    fn build_string_eq(
        &mut self,
        lhs: PointerValue<'ctx>,
        rhs: PointerValue<'ctx>,
        name: &str,
        negate: bool,
    ) -> Result<IntValue<'ctx>, String> {
        let function = self
            .builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .ok_or_else(|| "no current function".to_string())?;

        let entry_block = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "no current basic block".to_string())?;

        let loop_block = self
            .context
            .append_basic_block(function, &format!("{}.str.loop", name));

        let advance_block = self
            .context
            .append_basic_block(function, &format!("{}.str.advance", name));

        let equal_block = self
            .context
            .append_basic_block(function, &format!("{}.str.equal", name));

        let not_equal_block = self
            .context
            .append_basic_block(function, &format!("{}.str.not_equal", name));

        let done_block = self
            .context
            .append_basic_block(function, &format!("{}.str.done", name));

        // Current block -> loop.
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|err| err.to_string())?;

        self.builder.position_at_end(loop_block);

        // Loop-carried pointers.
        let lhs_phi = self
            .builder
            .build_phi(
                self.context.ptr_type(inkwell::AddressSpace::default()),
                &format!("{}.lhs", name),
            )
            .map_err(|err| err.to_string())?;

        let rhs_phi = self
            .builder
            .build_phi(
                self.context.ptr_type(inkwell::AddressSpace::default()),
                &format!("{}.rhs", name),
            )
            .map_err(|err| err.to_string())?;

        lhs_phi.add_incoming(&[(&lhs, entry_block)]);
        rhs_phi.add_incoming(&[(&rhs, entry_block)]);

        let lhs_ptr = lhs_phi.as_basic_value().into_pointer_value();
        let rhs_ptr = rhs_phi.as_basic_value().into_pointer_value();

        // Load the current characters.
        let lhs_byte = self
            .builder
            .build_load(
                self.context.i8_type(),
                lhs_ptr,
                &format!("{}.lhs.byte", name),
            )
            .map_err(|err| err.to_string())?
            .into_int_value();

        let rhs_byte = self
            .builder
            .build_load(
                self.context.i8_type(),
                rhs_ptr,
                &format!("{}.rhs.byte", name),
            )
            .map_err(|err| err.to_string())?
            .into_int_value();

        // If the bytes differ, the strings differ.
        let bytes_equal = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                lhs_byte,
                rhs_byte,
                &format!("{}.bytes.equal", name),
            )
            .map_err(|err| err.to_string())?;

        self.builder
            .build_conditional_branch(bytes_equal, advance_block, not_equal_block)
            .map_err(|err| err.to_string())?;

        // Bytes were equal. Check for the terminating '\0'.
        self.builder.position_at_end(advance_block);

        let lhs_end = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                lhs_byte,
                self.context.i8_type().const_zero(),
                &format!("{}.lhs.end", name),
            )
            .map_err(|err| err.to_string())?;

        let rhs_end = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                rhs_byte,
                self.context.i8_type().const_zero(),
                &format!("{}.rhs.end", name),
            )
            .map_err(|err| err.to_string())?;

        let either_end = self
            .builder
            .build_or(lhs_end, rhs_end, &format!("{}.either.end", name))
            .map_err(|err| err.to_string())?;

        // If either string ended, they are equal because the bytes
        // were already proven equal.
        let advance_or_equal = self
            .context
            .append_basic_block(function, &format!("{}.str.advance.bytes", name));

        self.builder
            .build_conditional_branch(either_end, equal_block, advance_or_equal)
            .map_err(|err| err.to_string())?;

        // Actually increment the pointers.
        self.builder.position_at_end(advance_or_equal);

        let one = self.context.i64_type().const_int(1, false);

        let next_lhs = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    lhs_ptr,
                    &[one],
                    &format!("{}.lhs.next", name),
                )
                .map_err(|err| err.to_string())?
        };

        let next_rhs = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    rhs_ptr,
                    &[one],
                    &format!("{}.rhs.next", name),
                )
                .map_err(|err| err.to_string())?
        };

        lhs_phi.add_incoming(&[(&next_lhs, advance_or_equal)]);
        rhs_phi.add_incoming(&[(&next_rhs, advance_or_equal)]);

        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|err| err.to_string())?;

        // Strings are equal.
        self.builder.position_at_end(equal_block);

        let equal_value = self
            .context
            .bool_type()
            .const_int(if negate { 0 } else { 1 }, false);

        self.builder
            .build_unconditional_branch(done_block)
            .map_err(|err| err.to_string())?;

        // Strings are not equal.
        self.builder.position_at_end(not_equal_block);

        let not_equal_value = self
            .context
            .bool_type()
            .const_int(if negate { 1 } else { 0 }, false);

        self.builder
            .build_unconditional_branch(done_block)
            .map_err(|err| err.to_string())?;

        // Merge the two results.
        self.builder.position_at_end(done_block);

        let result = self
            .builder
            .build_phi(self.context.bool_type(), name)
            .map_err(|err| err.to_string())?;

        result.add_incoming(&[
            (&equal_value, equal_block),
            (&not_equal_value, not_equal_block),
        ]);

        Ok(result.as_basic_value().into_int_value())
    }

    fn struct_field_at_offset(&self, struct_name: &str, offset: i64) -> Result<&Type, String> {
        let layout = self
            .struct_defs
            .get(struct_name)
            .ok_or_else(|| format!("unknown struct '{}'", struct_name))?;

        layout
            .field_offsets
            .values()
            .find(|(field_offset, _)| *field_offset == offset)
            .map(|(_, ty)| ty)
            .ok_or_else(|| format!("struct '{}' has no field at offset {}", struct_name, offset))
    }

    fn llvm_address_of(&mut self, value: &Value) -> Result<PointerValue<'ctx>, String> {
        match value {
            Value::Var(name) => {
                if let Some(ptr) = self.vars.get(name).copied() {
                    return Ok(ptr);
                }

                let ty = self
                    .var_types
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("unknown variable '{}'", name))?;

                let llvm_ty = self.llvm_type(&ty);

                let ptr = self
                    .builder
                    .build_alloca(llvm_ty, name)
                    .map_err(|err| err.to_string())?;

                self.vars.insert(name.clone(), ptr);

                Ok(ptr)
            }

            Value::Temp(name) => {
                let value = self
                    .temps
                    .get(name)
                    .copied()
                    .ok_or_else(|| format!("unknown temporary '{}'", name))?;

                if !value.is_pointer_value() {
                    return Err(format!("temporary '{}' is not a pointer", name));
                }

                Ok(value.into_pointer_value())
            }

            _ => Err("cannot take address of this value".to_string()),
        }
    }

    fn declare_structs(&mut self) -> Result<(), String> {
        // First create all named struct types.
        for name in self.struct_defs.keys() {
            let struct_type = self.context.opaque_struct_type(name);
            self.struct_types.insert(name.clone(), struct_type);
        }

        // Then populate their bodies.
        for (name, layout) in &self.struct_defs {
            let struct_type = self
                .struct_types
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown struct '{}'", name))?;

            let field_types = layout
                .field_offsets
                .values()
                .map(|(_, ty)| self.llvm_type(ty))
                .collect::<Vec<_>>();

            struct_type.set_body(&field_types, false);
        }

        Ok(())
    }
}

/// Compilation functions
impl<'ctx> LlvmBackend<'ctx> {
    #[allow(unreachable_patterns)]
    fn compile_instruction(&mut self, instruction: &Instruction) -> Result<(), String> {
        match instruction {
            Instruction::FunctionLabel(label) => self.compile_function_label(label),
            Instruction::Return { value } => self.compile_return(value),
            Instruction::Label(label) => self.compile_label(label),
            Instruction::Assign { dst, src } => self.compile_assign(dst, src),
            Instruction::Binary { dst, op, lhs, rhs } => self.compile_binary(dst, op, lhs, rhs),
            Instruction::Cast {
                dst,
                cast_ty,
                value,
                to_type,
            } => self.compile_cast(dst, cast_ty, value, to_type),
            Instruction::Jump(label) => self.compile_jump(label),
            Instruction::JumpIfFalse { cond, target } => self.compile_jumpiffalse(cond, target),
            Instruction::Extern { .. } => Ok(()),
            Instruction::Arg { value } => self.compile_arg(value),
            Instruction::Call { dest, name, argc } => self.compile_call(dest, name, *argc),
            Instruction::Param { p } => self.compile_param(p),
            Instruction::Unary { dst, op, value } => self.compile_unary(dst, op, value),
            Instruction::Load { dst, ptr, ty } => self.compile_load(dst, ptr, ty),
            Instruction::Store { ptr, source } => self.compile_store(ptr, source),
            _ => Err("unimplemented instruction, check back later :>".to_string()),
        }
    }

    fn compile_store(&mut self, ptr: &Value, source: &Value) -> Result<(), String> {
        let ptr_ty = self.value_type(ptr)?;
        let source_ty = self.value_type(source)?;

        let pointee_ty = match &ptr_ty {
            Type::Ptr(inner) => inner.as_ref(),
            _ => {
                return Err(format!(
                    "cannot store through non-pointer type {}",
                    type_to_string(&ptr_ty)
                ));
            }
        };

        if !types_equal(pointee_ty, &source_ty) {
            return Err(format!(
                "cannot store {} into pointer to {}",
                type_to_string(&source_ty),
                type_to_string(pointee_ty)
            ));
        }

        let ptr = self.llvm_ptr(ptr)?;
        let value = self.llvm_value(source)?;

        self.builder
            .build_store(ptr, value)
            .map_err(|err| err.to_string())?;

        Ok(())
    }

    fn compile_load(&mut self, dst: &str, ptr: &Value, ty: &Type) -> Result<(), String> {
        let ptr_ty = self.value_type(ptr)?;

        let pointee_ty = match &ptr_ty {
            Type::Ptr(inner) => inner.as_ref(),
            _ => {
                return Err(format!(
                    "cannot load through non-pointer type {}",
                    type_to_string(&ptr_ty)
                ));
            }
        };

        if !types_equal(pointee_ty, ty) {
            return Err(format!(
                "cannot load {} from pointer to {}",
                type_to_string(ty),
                type_to_string(pointee_ty)
            ));
        }

        let ptr = self.llvm_ptr(ptr)?;
        let llvm_ty = self.llvm_type(ty);

        let value = self
            .builder
            .build_load(llvm_ty, ptr, dst)
            .map_err(|err| err.to_string())?;

        self.temps.insert(dst.to_owned(), value);
        self.temp_types.insert(dst.to_owned(), ty.clone());

        Ok(())
    }

    fn compile_unary(&mut self, dst: &str, op: &IrOp, value: &Value) -> Result<(), String> {
        let value_type = self.value_type(value)?;

        let (result_val, result_type): (BasicValueEnum<'ctx>, Type) = match op {
            IrOp::Neg => {
                if !is_integer(&value_type) {
                    return Err(format!(
                        "cannot negate value of type {}",
                        type_to_string(&value_type)
                    ));
                }

                let value = self.llvm_value(value)?.into_int_value();

                (
                    self.builder
                        .build_int_neg(value, dst)
                        .map_err(|err| err.to_string())?
                        .into(),
                    value_type,
                )
            }

            IrOp::Pos => {
                if !is_integer(&value_type) {
                    return Err(format!(
                        "cannot apply unary + to value of type {}",
                        type_to_string(&value_type)
                    ));
                }

                (self.llvm_value(value)?, value_type)
            }

            IrOp::Not => {
                if !is_integer(&value_type) {
                    return Err(format!(
                        "cannot apply ~ to value of type {}",
                        type_to_string(&value_type)
                    ));
                }

                let value = self.llvm_value(value)?.into_int_value();

                (
                    self.builder
                        .build_not(value, dst)
                        .map_err(|err| err.to_string())?
                        .into(),
                    value_type,
                )
            }

            IrOp::Ref => (
                self.llvm_address_of(value)?.into(),
                Type::Ptr(Box::new(value_type)),
            ),

            _ => unreachable!(),
        };

        self.temps.insert(dst.to_owned(), result_val);
        self.temp_types.insert(dst.to_owned(), result_type);

        Ok(())
    }

    fn compile_param(&mut self, p: &str) -> Result<(), String> {
        let function_name = self.current_function_name()?;

        let function = self
            .functions
            .get(&function_name)
            .copied()
            .ok_or_else(|| format!("unknown function '{}'", function_name))?;

        let param_value = function
            .get_nth_param(self.current_param_idx)
            .ok_or_else(|| {
                format!(
                    "function '{}' has no parameter at index {}",
                    function_name, self.current_param_idx
                )
            })?;

        self.current_param_idx += 1;

        let ty = self
            .var_types
            .get(p)
            .cloned()
            .ok_or_else(|| format!("unknown variable '{}'", p))?;

        match ty {
            Type::Ptr(_) => {
                let ptr = param_value.into_pointer_value();

                self.vars.insert(p.to_string(), ptr);
            }

            ty => {
                let llvm_type = self.llvm_type(&ty);

                let ptr = self
                    .builder
                    .build_alloca(llvm_type, p)
                    .map_err(|err| err.to_string())?;

                self.builder
                    .build_store(ptr, param_value)
                    .map_err(|err| err.to_string())?;

                self.vars.insert(p.to_string(), ptr);
            }
        }

        Ok(())
    }

    fn compile_call(
        &mut self,
        dest: &Option<String>,
        name: &str,
        argc: usize,
    ) -> Result<(), String> {
        let function = self
            .functions
            .get(name)
            .copied()
            .ok_or_else(|| format!("unknown function '{}'", name))?;

        if self.pending_args.len() < argc {
            return Err(format!(
                "call to '{}' expected {} args, only {} staged",
                name,
                argc,
                self.pending_args.len()
            ));
        }

        let split_at = self.pending_args.len() - argc;
        let args: Vec<inkwell::values::BasicMetadataValueEnum> = self
            .pending_args
            .split_off(split_at)
            .into_iter()
            .map(Into::into)
            .collect();

        let call_name = dest.as_deref().unwrap_or("");
        let call_site = self
            .builder
            .build_call(function, &args, call_name)
            .map_err(|err| err.to_string())?;

        if let Some(dst) = dest {
            let ret_val = call_site
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| format!("call to '{}' used as a value but returns void", name))?;
            let sig = self
                .func_defs
                .get(name)
                .ok_or_else(|| format!("unknown function signature '{}'", name))?;

            self.temps.insert(dst.clone(), ret_val);
            self.temp_types.insert(dst.clone(), sig.return_type.clone());
        }

        Ok(())
    }

    fn compile_arg(&mut self, value: &Value) -> Result<(), String> {
        let llvm_value = self.llvm_value(value)?;
        self.pending_args.push(llvm_value);
        Ok(())
    }

    fn compile_jumpiffalse(&mut self, cond: &Value, target: &str) -> Result<(), String> {
        let target_block = self.get_block(target)?;

        let cond = self.llvm_truthy(cond)?;

        let current_block = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "no current basic block".to_string())?;

        let function = current_block
            .get_parent()
            .ok_or_else(|| "current block has no parent function".to_string())?;

        let continue_name = self.fresh_block_name("jumpiffalse.continue");

        let continue_block = self.context.append_basic_block(function, &continue_name);

        self.builder
            .build_conditional_branch(cond, continue_block, target_block)
            .map_err(|err| err.to_string())?;

        self.builder.position_at_end(continue_block);

        Ok(())
    }

    fn compile_jump(&mut self, target: &str) -> Result<(), String> {
        let block = self.get_block(target)?;

        self.builder
            .build_unconditional_branch(block)
            .map_err(|err| err.to_string())?;

        Ok(())
    }

    fn compile_cast(
        &mut self,
        dst: &str,
        cast_ty: &CastType,
        value: &Value,
        to_type: &Type,
    ) -> Result<(), String> {
        let from_type = self.value_type(value)?;

        if !is_integer(&from_type) || !is_integer(to_type) {
            return Err(format!(
                "ICE: integer cast from {} to {} reached LLVM backend",
                type_to_string(&from_type),
                type_to_string(to_type),
            ));
        }

        let value = self.llvm_value(value)?.into_int_value();
        let llvm_to_type = self.llvm_type(to_type).into_int_type();

        let result = match cast_ty {
            CastType::Extend => {
                if is_signed_integer(&from_type) {
                    self.builder
                        .build_int_s_extend(value, llvm_to_type, dst)
                        .map_err(|err| err.to_string())?
                } else {
                    self.builder
                        .build_int_z_extend(value, llvm_to_type, dst)
                        .map_err(|err| err.to_string())?
                }
            }

            CastType::Truncate => self
                .builder
                .build_int_truncate(value, llvm_to_type, dst)
                .map_err(|err| err.to_string())?,

            CastType::BitCast => self
                .builder
                .build_bit_cast(value, llvm_to_type, dst)
                .map_err(|err| err.to_string())?
                .into_int_value(),
        };

        self.temps.insert(dst.to_string(), result.into());
        self.temp_types.insert(dst.to_string(), to_type.clone());

        Ok(())
    }

    fn compile_binary(
        &mut self,
        dst: &str,
        op: &IrOp,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), String> {
        match op {
            IrOp::Add | IrOp::Sub | IrOp::Div | IrOp::Mul | IrOp::Mod => {
                self.compile_binary_maths(dst, op, lhs, rhs)
            }

            IrOp::Eq
            | IrOp::NEq
            | IrOp::Gt
            | IrOp::GtE
            | IrOp::Lt
            | IrOp::LtE
            | IrOp::And
            | IrOp::Or => self.compile_binary_comparison(dst, op, lhs, rhs),

            _ => unreachable!(),
        }
    }

    fn compile_binary_maths(
        &mut self,
        dst: &str,
        op: &IrOp,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), String> {
        let result_type = self.value_type(lhs)?;

        if matches!(self.value_type(lhs)?, Type::Ptr(_)) && is_integer(&self.value_type(rhs)?) {
            return self.compile_pointer_arithmetic(dst, op, lhs, rhs);
        }

        if !is_integer(&result_type) {
            return Err(format!(
                "ICE: non-integer type {} reached integer binary operation",
                type_to_string(&result_type)
            ));
        }

        let lhs_value = self.llvm_value(lhs)?.into_int_value();
        let rhs_value = self.llvm_value(rhs)?.into_int_value();

        let result = match op {
            IrOp::Add => self
                .builder
                .build_int_add(lhs_value, rhs_value, dst)
                .map_err(|err| err.to_string())?
                .into(),

            IrOp::Sub => self
                .builder
                .build_int_sub(lhs_value, rhs_value, dst)
                .map_err(|err| err.to_string())?
                .into(),

            IrOp::Mul => self
                .builder
                .build_int_mul(lhs_value, rhs_value, dst)
                .map_err(|err| err.to_string())?
                .into(),

            IrOp::Div => {
                if is_signed_integer(&result_type) {
                    self.builder
                        .build_int_signed_div(lhs_value, rhs_value, dst)
                        .map_err(|err| err.to_string())?
                        .into()
                } else {
                    self.builder
                        .build_int_unsigned_div(lhs_value, rhs_value, dst)
                        .map_err(|err| err.to_string())?
                        .into()
                }
            }

            IrOp::Mod => {
                if is_signed_integer(&result_type) {
                    self.builder
                        .build_int_signed_rem(lhs_value, rhs_value, dst)
                        .map_err(|err| err.to_string())?
                        .into()
                } else {
                    self.builder
                        .build_int_unsigned_rem(lhs_value, rhs_value, dst)
                        .map_err(|err| err.to_string())?
                        .into()
                }
            }

            _ => unreachable!(),
        };

        self.temps.insert(dst.to_string(), result);
        self.temp_types.insert(dst.to_string(), result_type);

        Ok(())
    }

    fn compile_pointer_arithmetic(
        &mut self,
        dst: &str,
        op: &IrOp,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), String> {
        let lhs_type = self.value_type(lhs)?;

        let result_pointee = match &lhs_type {
            Type::Ptr(inner) => match inner.as_ref() {
                Type::Struct(name) => {
                    if let Value::Const(offset) = rhs {
                        self.struct_field_at_offset(name, *offset)?.clone()
                    } else {
                        inner.as_ref().clone()
                    }
                }

                other => other.clone(),
            },

            _ => unreachable!(),
        };
        let ptr = self.llvm_value(lhs)?.into_pointer_value();
        let offset = self.llvm_value(rhs)?.into_int_value();

        let result = match op {
            IrOp::Add => unsafe {
                self.builder
                    .build_gep(self.context.i8_type(), ptr, &[offset], dst)
                    .map_err(|err| err.to_string())?
            },

            IrOp::Sub => {
                let neg = self
                    .builder
                    .build_int_neg(offset, &format!("{}.neg", dst))
                    .map_err(|err| err.to_string())?;

                unsafe {
                    self.builder
                        .build_gep(self.context.i8_type(), ptr, &[neg], dst)
                        .map_err(|err| err.to_string())?
                }
            }

            _ => {
                return Err(format!("invalid pointer arithmetic operation {:?}", op));
            }
        };

        self.temps.insert(dst.to_string(), result.into());
        self.temp_types
            .insert(dst.to_string(), Type::Ptr(Box::new(result_pointee.clone())));

        Ok(())
    }

    fn compile_binary_comparison(
        &mut self,
        dst: &str,
        op: &IrOp,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), String> {
        let result_type = self.value_type(lhs)?;

        if !types_equal(&result_type, &self.value_type(rhs)?) {
            return Err(format!(
                "cannot compare {} with {}",
                type_to_string(&result_type),
                type_to_string(&self.value_type(rhs)?)
            ));
        }

        if result_type == Type::Str {
            let lhs_value = self.llvm_value(lhs)?.into_pointer_value();
            let rhs_value = self.llvm_value(rhs)?.into_pointer_value();

            let result = match op {
                IrOp::Eq => self.build_string_eq(lhs_value, rhs_value, dst, false)?,
                IrOp::NEq => self.build_string_eq(lhs_value, rhs_value, dst, true)?,

                _ => {
                    return Err(format!("string operation {:?} not implemented", op));
                }
            };

            self.temps.insert(dst.to_string(), result.into());
            self.temp_types.insert(dst.to_string(), Type::Bool);

            return Ok(());
        }

        if !is_truthy_type(&result_type) {
            return Err(format!(
                "ICE: non-truthy type {} reached comparison binary operation",
                type_to_string(&result_type)
            ));
        }

        let lhs_value = self.llvm_value(lhs)?.into_int_value();
        let rhs_value = self.llvm_value(rhs)?.into_int_value();

        let result = match op {
            IrOp::Eq => self.build_int_compare(IntPredicate::EQ, lhs_value, rhs_value, dst)?,

            IrOp::NEq => self.build_int_compare(IntPredicate::NE, lhs_value, rhs_value, dst)?,

            IrOp::Gt => self.build_int_compare(
                if is_signed_integer(&result_type) {
                    IntPredicate::SGT
                } else {
                    IntPredicate::UGT
                },
                lhs_value,
                rhs_value,
                dst,
            )?,

            IrOp::GtE => self.build_int_compare(
                if is_signed_integer(&result_type) {
                    IntPredicate::SGE
                } else {
                    IntPredicate::UGE
                },
                lhs_value,
                rhs_value,
                dst,
            )?,

            IrOp::Lt => self.build_int_compare(
                if is_signed_integer(&result_type) {
                    IntPredicate::SLT
                } else {
                    IntPredicate::ULT
                },
                lhs_value,
                rhs_value,
                dst,
            )?,

            IrOp::LtE => self.build_int_compare(
                if is_signed_integer(&result_type) {
                    IntPredicate::SLE
                } else {
                    IntPredicate::ULE
                },
                lhs_value,
                rhs_value,
                dst,
            )?,

            IrOp::And => self
                .builder
                .build_and(lhs_value, rhs_value, dst)
                .map_err(|e| e.to_string())?
                .into(),

            IrOp::Or => self
                .builder
                .build_or(lhs_value, rhs_value, dst)
                .map_err(|e| e.to_string())?
                .into(),

            _ => {
                return Err(format!("binary operation {:?} not implemented yet", op));
            }
        };

        self.temps.insert(dst.to_string(), result);
        self.temp_types.insert(dst.to_string(), Type::Bool);

        Ok(())
    }
    fn compile_assign(&mut self, dst: &str, src: &Value) -> Result<(), String> {
        let dst_ty = self
            .var_types
            .get(dst)
            .cloned()
            .ok_or_else(|| format!("unknown variable '{}'", dst))?;

        let src_ty = self.value_type(src)?;

        if !types_equal(&dst_ty, &src_ty) {
            return Err(format!(
                "cannot assign {} to {} of type {}",
                type_to_string(&src_ty),
                dst,
                type_to_string(&dst_ty)
            ));
        }

        let llvm_value = self.llvm_value(src)?;
        let llvm_type = self.llvm_type(&dst_ty);

        let ptr = match self.vars.get(dst) {
            Some(ptr) => *ptr,

            None => {
                let ptr = self
                    .builder
                    .build_alloca(llvm_type, dst)
                    .map_err(|err| err.to_string())?;

                self.vars.insert(dst.to_string(), ptr);
                ptr
            }
        };

        self.builder
            .build_store(ptr, llvm_value)
            .map_err(|err| err.to_string())?;

        Ok(())
    }

    fn compile_function_label(&mut self, label: &str) -> Result<(), String> {
        let entry = self
            .blocks
            .get(&format!("{}::entry", label))
            .copied()
            .ok_or_else(|| format!("function '{}' has no entry block", label))?;

        self.builder.position_at_end(entry);
        self.current_param_idx = 0;

        Ok(())
    }

    fn compile_return(&mut self, value: &Value) -> Result<(), String> {
        match value {
            Value::Void => {
                self.builder
                    .build_return(None)
                    .map_err(|err| err.to_string())?;
            }

            _ => {
                let value = self.llvm_value(value)?;

                self.builder
                    .build_return(Some(&value))
                    .map_err(|err| err.to_string())?;
            }
        }

        Ok(())
    }

    fn compile_label(&mut self, label: &str) -> Result<(), String> {
        let block = self.get_block(label)?;

        if let Some(current) = self.builder.get_insert_block()
            && current != block
            && current.get_terminator().is_none()
        {
            self.builder
                .build_unconditional_branch(block)
                .map_err(|err| err.to_string())?;
        }

        self.builder.position_at_end(block);

        Ok(())
    }
}

/// preprocessing processes
impl<'ctx> LlvmBackend<'ctx> {
    fn create_blocks(&mut self, instructions: &[Instruction]) -> Result<(), String> {
        let mut current_function: Option<FunctionValue<'ctx>> = None;
        let mut current_function_name: Option<String> = None;

        for instruction in instructions {
            match instruction {
                Instruction::FunctionLabel(name) => {
                    let function = self
                        .functions
                        .get(name)
                        .copied()
                        .ok_or_else(|| format!("unknown function '{}'", name))?;

                    let entry = self.context.append_basic_block(function, "entry");

                    self.blocks.insert(format!("{}::entry", name), entry);

                    current_function = Some(function);
                    current_function_name = Some(name.clone());
                }

                Instruction::Label(label) => {
                    let function =
                        current_function.ok_or_else(|| "label outside function".to_string())?;

                    let function_name = current_function_name
                        .as_ref()
                        .ok_or_else(|| "missing current function name".to_string())?;

                    let block = self.context.append_basic_block(function, label);

                    self.blocks
                        .insert(format!("{}::{}", function_name, label), block);
                }

                _ => {}
            }
        }

        Ok(())
    }

    fn declare_functions(&mut self, tac: &[Instruction]) -> Result<(), String> {
        // First collect the functions that actually occur in the TAC.
        let mut tac_functions: Vec<String> = Vec::new();

        for instruction in tac {
            if let Instruction::FunctionLabel(name) = instruction
                && !tac_functions.contains(name)
            {
                tac_functions.push(name.clone());
            }
        }

        // Declare every concrete function emitted by TAC.
        for name in tac_functions {
            if self.functions.contains_key(&name) {
                continue;
            }

            let mut llvm_param_types = Vec::new();

            for instruction in tac {
                let param_name = match instruction {
                    Instruction::Param { p } => p,
                    _ => continue,
                };

                let prefix = format!("{}::", name);

                if !param_name.starts_with(&prefix) {
                    continue;
                }

                let param_ty = self
                    .var_types
                    .get(param_name)
                    .ok_or_else(|| format!("no type information for parameter '{}'", param_name))?;

                llvm_param_types.push(self.llvm_type(param_ty).into());
            }

            // If this concrete TAC function has no Param instructions,
            // fall back to the semantic signature. This handles things
            // like main() and zero-parameter functions.
            if llvm_param_types.is_empty()
                && let Some(sig) = self.func_defs.get(&name)
            {
                llvm_param_types = sig
                    .param_types
                    .iter()
                    .map(|ty| self.llvm_type(ty).into())
                    .collect();
            }

            let return_type = self
                .func_defs
                .get(&name)
                .map(|sig| sig.return_type.clone())
                .unwrap_or(Type::Void);

            let fn_type = match &return_type {
                Type::Void => self.context.void_type().fn_type(&llvm_param_types, false),

                ty => self.llvm_type(ty).fn_type(&llvm_param_types, false),
            };

            let function = self.module.add_function(&name, fn_type, None);

            self.functions.insert(name, function);
        }

        // Declare externs that don't have TAC FunctionLabels.
        for (name, sig) in &self.func_defs {
            if self.functions.contains_key(name) {
                continue;
            }

            // Only functions with no TAC body reach here.
            let param_types: Vec<_> = sig
                .param_types
                .iter()
                .map(|ty| self.llvm_type(ty).into())
                .collect();

            let fn_type = match &sig.return_type {
                Type::Void => self.context.void_type().fn_type(&param_types, false),

                ty => self.llvm_type(ty).fn_type(&param_types, false),
            };

            let function = self.module.add_function(name, fn_type, None);

            self.functions.insert(name.clone(), function);
        }

        Ok(())
    }
}
