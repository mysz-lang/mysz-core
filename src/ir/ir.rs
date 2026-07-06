use std::collections::HashMap;

use crate::{
    ir::tac::{Instruction, IrOp, Value}, 
    parse::parsing::{
        BinaryOp, Expr, ExprKind, Literal, Parameter, Program, Stmt, UnaryOp, Type,
    },
};

pub struct TempGen {
    counter: usize,
}

impl TempGen {
    pub fn new() -> Self {
        Self { counter: 0 }
    }

    pub fn next(&mut self) -> String {
        self.counter += 1;
        format!("t{}", self.counter)
    }
}

pub struct LabelGen {
    counter: usize,
}

impl LabelGen {
    pub fn new() -> Self {
        Self { counter: 0 }
    }

    pub fn next(&mut self) -> String {
        self.counter += 1;
        format!("L{}", self.counter)
    }
}

pub struct FunctionGen {
    counter: usize,
}

impl FunctionGen {
    pub fn new() -> Self {
        Self { counter: 0 }
    }

    pub fn next(&mut self, name: String) -> String {
        self.counter += 1;
        name
    }
}

pub struct IRGen {
    pub code: Vec<Instruction>,
    temps: TempGen,
    labels: LabelGen,
    functions: FunctionGen,
    var_types: HashMap<String, Type>,
}

impl IRGen {
    pub fn new(types: HashMap<String, Type>) -> Self {
        Self {
            code: Vec::new(),
            temps: TempGen::new(),
            labels: LabelGen::new(),
            functions: FunctionGen::new(),
            var_types: types
        }
    }

    fn emit_binary(
        &mut self,
        op: IrOp,
        lhs: Value,
        rhs: Value,
    ) -> Value {
        let temp = self.temps.next();

        self.code.push(Instruction::Binary {
            dst: temp.clone(),
            op,
            lhs,
            rhs,
        });

        Value::Temp(temp)
    }

    fn emit_unary(
        &mut self,
        op: IrOp,
        value: Value,
    ) -> Value {
        let temp = self.temps.next();

        self.code.push(Instruction::Unary {
            dst: temp.clone(),
            op,
            value,
        });

        Value::Temp(temp)
    }

    fn is_string_valued(&self, value: &Value) -> bool {
        matches!(value, Value::Str(_))
    }

    fn expr_type(&self, expr: &Expr) -> Option<Type> {
        match &expr.kind {
            ExprKind::Literal(Literal::String(_)) => Some(Type::Str),
            ExprKind::Literal(Literal::Int(_)) => Some(Type::Int),
            ExprKind::Literal(Literal::Bool(_)) => Some(Type::Bool),
            ExprKind::Identifier(name) => self.var_types.get(name).cloned(),
            ExprKind::Binary { left, .. } => self.expr_type(left), 
            ExprKind::Call { .. } => None, 
            
            ExprKind::Index { base, .. } => {
                match self.expr_type(base)? {
                    Type::Array { element_type, .. } => Some(*element_type),
                    Type::Ptr(inner) => match *inner {
                        Type::Array { element_type, .. } => Some(*element_type),
                        other => Some(other),
                    },
                    _ => None,
                }
            }

            ExprKind::Unary { op, expr: inner_expr } => {
                let inner_type = self.expr_type(inner_expr)?;
                match op {
                    UnaryOp::AddressOf => Some(Type::Ptr(Box::new(inner_type))),
                    UnaryOp::Deref => match inner_type {
                        Type::Ptr(inner) => Some(*inner),
                        _ => None,
                    },
                    UnaryOp::Positive | UnaryOp::Negative => Some(Type::Int),
                }
            }
            _ => None,
        }
    }

    pub fn gen_expr(&mut self, expr: &Expr, target_dest: Option<Value>) -> Value {
        match &expr.kind {
            ExprKind::Literal(lit) => match lit {
                Literal::Int(v) => Value::Const(*v),
                Literal::String(s) => Value::Str(s.clone()),
                Literal::Bool(b) => Value::Bool(*b),
                Literal::Arr { elements } => {
                    let base_val = match target_dest {
                        Some(dest) => dest,
                        None => {
                            let anon_name = format!("_anon_{}", self.temps.next());
                            Value::Var(anon_name)
                        }
                    };

                    for (index, element_expr) in elements.iter().enumerate() {
                        let element_val = self.gen_expr(element_expr, None);
                        
                        let offset_temp = self.temps.next();
                        self.code.push(Instruction::Binary {
                            dst: offset_temp.clone(),
                            op: IrOp::Mul,
                            lhs: Value::Const(index as i64),
                            rhs: Value::Const(8),
                        });
                        
                        let base_addr_temp = self.temps.next();
                        self.code.push(Instruction::Unary {
                            dst: base_addr_temp.clone(),
                            op: IrOp::Ref,
                            value: base_val.clone(),
                        });
                        
                        let slot_addr_temp = self.temps.next();
                        self.code.push(Instruction::Binary {
                            dst: slot_addr_temp.clone(),
                            op: IrOp::Add,
                            lhs: Value::Temp(base_addr_temp),
                            rhs: Value::Temp(offset_temp),
                        });
                        
                        self.code.push(Instruction::Store {
                            ptr: Value::Temp(slot_addr_temp),
                            source: element_val,
                        });
                    }
                    
                    base_val
                }
            }

            ExprKind::Index { base, index } => {
                let base_val = self.gen_expr(base, None);
                let index_val = self.gen_expr(index, None);
                let offset_temp = self.temps.next();
                self.code.push(Instruction::Binary {
                    dst: offset_temp.clone(),
                    op: IrOp::Mul,
                    lhs: index_val,
                    rhs: Value::Const(8),
                });
                
                let base_type = self.expr_type(base); 
                let target_addr_temp = self.temps.next();

                let element_type = match &base_type {
                    Some(Type::Array { element_type, .. }) => *element_type.clone(),
                    Some(Type::Ptr(inner)) => match &**inner {
                        Type::Array { element_type, .. } => *element_type.clone(),
                        other => other.clone(),
                    }
                    _ => Type::Int,
                };

                let is_base_variable_a_pointer = match &base_val {
                    Value::Var(name) => matches!(self.var_types.get(name), Some(Type::Ptr(_))),
                    _ => false,
                };

                match base_type {
                    Some(Type::Array { .. }) => {
                        // If the underlying storage variable is a pointer, its value IS the address.
                        if is_base_variable_a_pointer {
                            self.code.push(Instruction::Binary {
                                dst: target_addr_temp.clone(),
                                op: IrOp::Add,
                                lhs: base_val, // Emits: Temp = Var("y") Add Offset
                                rhs: Value::Temp(offset_temp),
                            });
                        } else {
                            // It's a normal local array variable like "x". We must get its stack address frame.
                            match base_val {
                                Value::Var(_) => {
                                    let base_addr_temp = self.temps.next();
                                    self.code.push(Instruction::Unary {
                                        dst: base_addr_temp.clone(),
                                        op: IrOp::Ref,
                                        value: base_val,
                                    });
                                    self.code.push(Instruction::Binary {
                                        dst: target_addr_temp.clone(),
                                        op: IrOp::Add,
                                        lhs: Value::Temp(base_addr_temp),
                                        rhs: Value::Temp(offset_temp),
                                    });
                                }
                                _ => {
                                    self.code.push(Instruction::Binary {
                                        dst: target_addr_temp.clone(),
                                        op: IrOp::Add,
                                        lhs: base_val,
                                        rhs: Value::Temp(offset_temp),
                                    });
                                }
                            }
                        }
                    }
                    Some(Type::Ptr(_)) => {
                        self.code.push(Instruction::Binary {
                            dst: target_addr_temp.clone(),
                            op: IrOp::Add,
                            lhs: base_val,
                            rhs: Value::Temp(offset_temp),
                        });
                    }
                    _ => {
                        // Fallback safe catch-all
                        if is_base_variable_a_pointer {
                            self.code.push(Instruction::Binary {
                                dst: target_addr_temp.clone(),
                                op: IrOp::Add,
                                lhs: base_val,
                                rhs: Value::Temp(offset_temp),
                            });
                        } else {
                            match base_val {
                                Value::Var(_) => {
                                    let base_addr_temp = self.temps.next();
                                    self.code.push(Instruction::Unary {
                                        dst: base_addr_temp.clone(),
                                        op: IrOp::Ref,
                                        value: base_val,
                                    });
                                    self.code.push(Instruction::Binary {
                                        dst: target_addr_temp.clone(),
                                        op: IrOp::Add,
                                        lhs: Value::Temp(base_addr_temp),
                                        rhs: Value::Temp(offset_temp),
                                    });
                                }
                                _ => {
                                    self.code.push(Instruction::Binary {
                                        dst: target_addr_temp.clone(),
                                        op: IrOp::Add,
                                        lhs: base_val,
                                        rhs: Value::Temp(offset_temp),
                                    });
                                }
                            }
                        }
                    }
                };

                let result_temp = self.temps.next();
                self.code.push(Instruction::Load {
                    dst: result_temp.clone(),
                    ptr: Value::Temp(target_addr_temp),
                    ty: element_type,
                });

                Value::Temp(result_temp)
            }

            ExprKind::Identifier(name) => Value::Var(name.clone()),

            ExprKind::Unary { op, expr } => {
                let value = self.gen_expr(expr, None);

                match op {
                    UnaryOp::Positive => self.emit_unary(IrOp::Pos, value),

                    UnaryOp::Negative => self.emit_unary(IrOp::Neg, value),


                    UnaryOp::Deref => {
                        value
                    }
                            
                    UnaryOp::AddressOf => {
                        let temp = self.temps.next();
                        
                        self.code.push(Instruction::Unary {
                            dst: temp.clone(),
                            op: IrOp::Ref,
                            value,
                        });
                        
                        Value::Temp(temp)
                    }
                }
            }

            ExprKind::Binary { left, op, right } => {
                let lhs = self.gen_expr(left, None);
                let rhs = self.gen_expr(right, None);

                if matches!(op, BinaryOp::Add) 
                    && (self.is_string_valued(&lhs) || self.expr_type(left) == Some(Type::Str)) 
                    && (self.is_string_valued(&rhs) || self.expr_type(right) == Some(Type::Str)) 
                {
                    self.code.push(Instruction::Arg { value: lhs });
                    self.code.push(Instruction::Arg { value: rhs });
                    let dst = self.temps.next();
                    self.code.push(Instruction::Call {
                        dest: Some(dst.clone()),
                        name: "str_concat".to_string(),
                        argc: 2,
                    });
                    return Value::Temp(dst);
                }

                let ir_op = match op {
                    BinaryOp::Add => IrOp::Add,
                    BinaryOp::Sub => IrOp::Sub,
                    BinaryOp::Mul => IrOp::Mul,
                    BinaryOp::Div => IrOp::Div,

                    BinaryOp::Eq => IrOp::Eq,
                    BinaryOp::NEq => IrOp::NEq,
                    BinaryOp::Gt => IrOp::Gt,
                    BinaryOp::GtE => IrOp::GtE,
                    BinaryOp::Lt => IrOp::Lt,
                    BinaryOp::LtE => IrOp::LtE,

                    BinaryOp::Mod => IrOp::Mod,
                };

                self.emit_binary(ir_op, lhs, rhs)
            }

            ExprKind::Call { callee, args } => {
                let arg_values: Vec<Value> = args.iter()
                    .map(|arg| self.gen_expr(arg, None))
                    .collect();

                for val in arg_values.iter() {
                    self.code.push(Instruction::Arg { value: val.clone() });
                }

                let dst = self.temps.next();

                self.code.push(Instruction::Call {
                    dest: Some(dst.clone()),
                    name: callee.value.clone(),
                    argc: arg_values.len(),
                });

                Value::Temp(dst)
            }
        }
    }

    pub fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {

            Stmt::Assignment { ident, vtype, expr } => {
                let is_array = match vtype {
                    Some(Type::Array { .. }) => true,
                    _ => false,
                };

                let target_var = Value::Var(ident.value.clone());
                
                if is_array {
                    self.gen_expr(expr, Some(target_var));
                } else {
                    let value = self.gen_expr(expr, None);
                    self.code.push(Instruction::Assign {
                        dst: ident.value.clone(),
                        src: value,
                    });
                }
            }
            Stmt::Reassignment { ident, expr } => {
                let value = self.gen_expr(expr, None);

                self.code.push(Instruction::Assign {
                    dst: ident.value.clone(),
                    src: value,
                });
            }

                Stmt::Expr(expr) => {
                if let ExprKind::Call { callee, args } = &expr.kind {
                    let arg_values: Vec<Value> = args.iter()
                        .map(|arg| self.gen_expr(arg, None))
                        .collect();

                    for val in arg_values.iter() {
                        self.code.push(Instruction::Arg { value: val.clone() });
                    }

                    self.code.push(Instruction::Call {
                        dest: None, 
                        name: callee.value.clone(),
                        argc: arg_values.len(),
                    });
                } else {
                    self.gen_expr(expr, None);
                }
            }

            Stmt::If {
                cond,
                then_branch,
                else_branch
            } => {
                let end = self.labels.next();

                let cond_val = self.gen_expr(cond, None);

                self.code.push(Instruction::JumpIfFalse {
                    cond: cond_val,
                    target: end.clone(),
                });

                for stmt in then_branch {
                    self.gen_stmt(stmt);
                }

                if else_branch.is_none() {
                    self.code.push(Instruction::Label(end));
                } else {
                    let true_end = self.labels.next();
                    self.code.push(Instruction::Jump(true_end.clone()));
                    self.code.push(Instruction::Label(end));
                    for stmt in else_branch.as_ref().unwrap() {
                        self.gen_stmt(stmt);
                    }
                    self.code.push(Instruction::Label(true_end));
                }
            }

            Stmt::While { cond, body } => {
                let start = self.labels.next();
                let end = self.labels.next();

                self.code.push(Instruction::Label(start.clone()));
                
                let cond_val = self.gen_expr(cond, None);
                self.code.push(Instruction::JumpIfFalse { cond: cond_val, target: end.clone() });

                for stmt in body {
                    self.gen_stmt(stmt);
                }
                self.code.push(Instruction::Jump(start));
                self.code.push(Instruction::Label(end));
            }

            Stmt::Function { name, params, body, .. } => {
                let start = self.functions.next(name.value.clone());
                self.code.push(Instruction::FunctionLabel(start));

                for param in params {
                    self.gen_param(param);
                }

                for stmt in body {
                    self.gen_stmt(stmt);
                }

                if !matches!(body.last(), Some(Stmt::Return { .. })) {
                    self.code.push(Instruction::Return { value: Value::Const(0) });
                }
            },

            Stmt::Return { value, .. } => {
                if let Some(expr) = value {
                    let val = self.gen_expr(expr, None);
                    self.code.push(Instruction::Return { value: val });
                } else {
                    self.code.push(Instruction::Return { value: Value::Const(0) })
                }
            },

            Stmt::Extern { name, .. } => {
                self.code.push(Instruction::Extern { fnname: name.value.clone() })
            }
            Stmt::DerefReassignment { target, expr } => {
                let value_to_store = self.gen_expr(expr, None);

                if let ExprKind::Index { base, index } = &target.kind {
                    let base_val = self.gen_expr(base, None);
                    let index_val = self.gen_expr(index, None);
                    
                    let offset_temp = self.temps.next();
                    self.code.push(Instruction::Binary {
                        dst: offset_temp.clone(),
                        op: IrOp::Mul,
                        lhs: index_val,
                        rhs: Value::Const(8), // Assuming 64-bit/8-byte elements
                    });
                    
                    let base_type = self.expr_type(base);
                    let target_addr_temp = self.temps.next();
                    
                    match base_type {
                        Some(Type::Array { .. }) => {
                            let base_addr_temp = self.temps.next();
                            self.code.push(Instruction::Unary {
                                dst: base_addr_temp.clone(),
                                op: IrOp::Ref,
                                value: base_val,
                            });
                            self.code.push(Instruction::Binary {
                                dst: target_addr_temp.clone(),
                                op: IrOp::Add,
                                lhs: Value::Temp(base_addr_temp),
                                rhs: Value::Temp(offset_temp),
                            });
                        }
                        Some(Type::Ptr(_)) => {
                            self.code.push(Instruction::Binary {
                                dst: target_addr_temp.clone(),
                                op: IrOp::Add,
                                lhs: base_val,
                                rhs: Value::Temp(offset_temp),
                            });
                        }
                        _ => {
                            panic!("Internal Compiler Error: Attempted IR index calculation on non-indexable type.");
                        }
                    };

                    self.code.push(Instruction::Store {
                        ptr: Value::Temp(target_addr_temp),
                        source: value_to_store,
                    });

                } else {

                    let target_ptr_val = self.gen_expr(target, None);

                    self.code.push(Instruction::Store {
                        ptr: target_ptr_val,
                        source: value_to_store,
                    });
                }
            }
        }
    }

    pub fn gen_param(&mut self, param: &Parameter) {
        self.code.push(Instruction::Param { p: param.name.value.clone() });
    }

    pub fn gen_program(&mut self, program: &Program) {
        for stmt in &program.statements {
            if !matches!(stmt, Stmt::Function { .. }) && !matches!(stmt, Stmt::Extern { .. }) {
                println!("Codegen Error: top-level statement outside of a function is not supported (wrap it in 'fn main() {{ ... }}')");
                std::process::exit(1);
            }
            self.gen_stmt(stmt);
        }
    }

    pub fn dump(&self) {
        for inst in &self.code {
            match inst {
                Instruction::Assign { dst, src } => {
                    println!("{dst} = {:?}", src);
                }
                Instruction::Binary { dst, op, lhs, rhs } => {
                    println!("{dst} = {:?} {:?} {:?}", lhs, op, rhs);
                }
                Instruction::Unary { dst, op, value } => {
                    println!("{dst} = {:?}{:?}", op, value);
                }
                Instruction::Label(label) => {
                    println!("{label}:");
                }
                Instruction::Jump(label) => {
                    println!("goto {label}");
                }
                Instruction::JumpIfFalse { cond, target } => {
                    println!("ifFalse {:?} goto {target}", cond);
                }
                Instruction::Param { p } => {
                    println!("param {}", p)
                }
                Instruction::FunctionLabel(label) => {
                    println!("{label}:")
                }
                Instruction::Return { value } => {
                    println!("return {:?}", value)
                }
                Instruction::Arg { value } => {
                    println!("arg {:?}", value)
                },
                Instruction::Call { dest, name, argc } => {
                    println!("call {:?} @ {:?} [arg_count: {}]", name, dest.clone().unwrap_or("n/a".to_string()), argc)
                },
                Instruction::Extern { fnname } => {
                    println!("extern {}", fnname)
                }
                Instruction::Store { ptr, source} => {
                    println!("store {:?} to *{:?}", source, ptr)
                }
                Instruction::Load { dst, ptr, ty } => {
                    println!("load {:?} [{:?}] from *{:?}", dst, ty, ptr)
                },
            }
        }
    }
}