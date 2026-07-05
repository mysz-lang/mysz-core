use std::collections::HashMap;

use crate::{
    ir::tac::{Instruction, IrOp, Value}, parse::parsing::{
        BinaryOp, Expr, ExprKind, Literal, Parameter, Program, Stmt, UnaryOp,
    }, semantics::analysis::Type,
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
        format!("{}", name)
    }
}

pub struct Codegen {
    pub instrs: Vec<Instruction>,
    pub temp_count: usize,
}

impl Codegen {
    fn new_temp(&mut self) -> String {
        self.temp_count += 1;
        format!("t{}", self.temp_count)
    }

    fn emit(&mut self, instr: Instruction) {
        self.instrs.push(instr);
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
            ExprKind::Identifier(name) => self.var_types.get(name).copied(),
            ExprKind::Binary { left, .. } => self.expr_type(left), // Add on Str yields Str, etc — simplistic but works for now
            ExprKind::Call { .. } => None, // would need function return-type lookup too
            _ => None,
        }
    }

    pub fn gen_expr(&mut self, expr: &Expr) -> Value {
        match &expr.kind {
            ExprKind::Literal(lit) => match lit {
                Literal::Int(v) => Value::Const(*v),
                Literal::String(s) => Value::Str(s.clone()),
                Literal::Bool(b) => Value::Bool(*b),
            },

            ExprKind::Identifier(name) => Value::Var(name.clone()),

            ExprKind::Unary { op, expr } => {
                let value = self.gen_expr(expr);

                let ir_op = match op {
                    UnaryOp::Positive => IrOp::Pos,
                    UnaryOp::Negative => IrOp::Neg,
                };

                self.emit_unary(ir_op, value)
            }

            ExprKind::Binary { left, op, right } => {
                let lhs = self.gen_expr(left);
                let rhs = self.gen_expr(right);

                if matches!(op, BinaryOp::Add) && (self.is_string_valued(&lhs) || self.expr_type(left) == Some(Type::Str)) && (self.is_string_valued(&rhs)|| self.expr_type(right) == Some(Type::Str)) {
                    self.code.push(Instruction::Arg { value: lhs });
                    self.code.push(Instruction::Arg { value: rhs });
                    let dst = self.temps.next();
                    self.code.push(Instruction::Call {
                        dest: Some(dst.clone()),
                        name: "__str_concat".to_string(),
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

                    BinaryOp::Mod => todo!("Modulo IR not implemented"),
                };

                self.emit_binary(ir_op, lhs, rhs)
            }

            ExprKind::Call { callee, args } => {
                let arg_values: Vec<Value> = args.iter()
                    .map(|arg| self.gen_expr(arg))
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
            Stmt::Assignment { ident, expr, .. }
            | Stmt::Reassignment { ident, expr } => {
                let value = self.gen_expr(expr);

                self.code.push(Instruction::Assign {
                    dst: ident.value.clone(),
                    src: value,
                });
            }

            Stmt::Expr(expr) => {
                self.gen_expr(expr);
            }

            // normal if:
            // iffalse cond goto end
            // [body]
            // end:
            // ---
            // if/else:
            // iffalse cond goto end
            // [then body]
            // goto true_end
            // end:
            Stmt::If {
                cond,
                then_branch,
                else_branch
            } => {
                let end = self.labels.next(); // l2

                let cond_val = self.gen_expr(cond);

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
                        self.gen_stmt(&stmt);
                    }
                    self.code.push(Instruction::Label(true_end));
                    
                }
            }

            Stmt::While { cond, body } => {
                let start = self.labels.next(); // l1
                let end = self.labels.next(); // l2

                self.code.push(Instruction::Label(start.clone()));
                
                let cond_val = self.gen_expr(cond); // cond
                self.code.push(Instruction::JumpIfFalse { cond: cond_val, target: end.clone() });

                for stmt in body {
                    self.gen_stmt(stmt);
                }
                self.code.push(Instruction::Jump(start));
                self.code.push(Instruction::Label(end));
            }
            Stmt::Function { name, rttype, params, body } => {
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
            Stmt::Return { value, span } => {
                if let Some(expr) = value {
                    let val = self.gen_expr(expr);
                    self.code.push(Instruction::Return { value: val });
                } else {
                    self.code.push(Instruction::Return { value: Value::Const(0) })
                }
            },
            Stmt::Extern { name, .. } => {
                self.code.push(Instruction::Extern { fnname: name.value.clone() })
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
            }
        }
    }
}