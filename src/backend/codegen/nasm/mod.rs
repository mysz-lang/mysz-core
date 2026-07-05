use std::collections::HashMap;

use crate::ir::tac::{Instruction, IrOp, Value};
use crate::backend::codegen::{Target, Backend};

fn split_functions(program: &[Instruction]) -> Vec<Vec<Instruction>> {
    let mut funcs = vec![];
    let mut current = vec![];

    for inst in program {
        match inst {
            Instruction::FunctionLabel(_) => {
                if !current.is_empty() {
                    funcs.push(current);
                    current = vec![];
                }
                current.push(inst.clone());
            }

            _ => current.push(inst.clone()),
        }
    }

    if !current.is_empty() {
        funcs.push(current);
    }

    funcs
}

#[derive(Default)]
struct StackFrame {
    slots: HashMap<String, i32>,
    next_offset: i32,
    size: i32,
}

impl StackFrame {
    fn new() -> Self {
        Self {
            slots: HashMap::new(),
            next_offset: 0,
            size: 0,
        }
    }

    fn alloc(&mut self, name: &str) {
        if self.slots.contains_key(name) {
            return;
        }
        self.next_offset += 8;
        self.slots.insert(name.to_string(), self.next_offset);
    }

    fn finalize(&mut self) {
        self.size = (self.next_offset + 15) & !15;
    }

    fn addr(&self, name: &str) -> String {
        match self.slots.get(name) {
            Some(off) => format!("[rbp - {}]", off),
            None => panic!("unallocated variable in function: {}", name),
        }
    }
}

pub struct FunctionCtx {
    arg_index: usize,
    param_index: usize,
}

pub struct NasmBackend {
    pub target: Target,
    pub out: String,
    frame: StackFrame,
    ctx: FunctionCtx,
}

impl NasmBackend {
    fn emit(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn reset(&mut self) {
        self.frame = StackFrame::new();
        self.ctx = FunctionCtx {
            arg_index: 0,
            param_index: 0,
        };
    }

    fn lower_value(&self, v: &Value) -> String {
        match v {
            Value::Const(i) => i.to_string(),
            Value::Bool(b) => if *b { "1".to_string() } else { "0".to_string() },
            Value::Void => "0".to_string(),
            Value::Temp(t) | Value::Var(t) => self.frame.addr(t),
            Value::Str(_) => panic!("strings not supported"),
        }
    }

    fn collect_value(&mut self, v: &Value) {
        match v {
            Value::Temp(t) | Value::Var(t) => self.frame.alloc(t),
            _ => {}
        }
    }

    fn collect_stack_slots(&mut self, program: &[Instruction]) {
        for inst in program {
            match inst {
                Instruction::Assign { dst, src } => {
                    self.frame.alloc(dst);
                    self.collect_value(src);
                }

                Instruction::Binary { dst, lhs, rhs, .. } => {
                    self.frame.alloc(dst);
                    self.collect_value(lhs);
                    self.collect_value(rhs);
                }

                Instruction::Unary { dst, value, .. } => {
                    self.frame.alloc(dst);
                    self.collect_value(value);
                }

                Instruction::Arg { value } => {
                    self.collect_value(value);
                }

                Instruction::Return { value } => {
                    self.collect_value(value);
                }

                Instruction::Call { dest, .. } => {
                    if let Some(d) = dest {
                        self.frame.alloc(d);
                    }
                }

                Instruction::Param { p } => {
                    self.frame.alloc(p);
                }

                _ => {}
            }
        }
    }

    fn emit_prologue(&mut self) {
        self.emit("push rbp");
        self.emit("mov rbp, rsp");

        if self.frame.size > 0 {
            self.emit(&format!("sub rsp, {}", self.frame.size));
        }
    }

    fn emit_instruction(&mut self, inst: &Instruction) {
        match inst {
            Instruction::FunctionLabel(name) => {
                // DON'T reset or collect slots here. It's already done in emit_program!
                self.emit(&format!("global {}", name));
                self.emit(&format!("{}:", name));

                // Just emit the prologue using the frame size calculated by emit_program
                self.emit_prologue();
            }

            Instruction::Label(l) => self.emit(&format!("{}:", l)),

            Instruction::Assign { dst, src } => {
                let dst = self.frame.addr(dst);
                self.emit(&format!("mov qword {}, {}", dst, self.lower_value(src)));
            }

            Instruction::Binary { dst, op, lhs, rhs } => {
                let dst = self.frame.addr(dst);

                self.emit(&format!("mov rax, {}", self.lower_value(lhs)));
                self.emit(&format!("mov rbx, {}", self.lower_value(rhs)));

                match op {
                    IrOp::Add => self.emit("add rax, rbx"),
                    IrOp::Sub => self.emit("sub rax, rbx"),
                    IrOp::Mul => self.emit("imul rax, rbx"),
                    IrOp::Div => {
                        self.emit("cqo");
                        self.emit("idiv rbx");
                    }
                    IrOp::Eq => {
                        self.emit("cmp rax, rbx");
                        self.emit("sete al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::NEq => {
                        self.emit("cmp rax, rbx");
                        self.emit("setne al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::Gt => {
                        self.emit("cmp rax, rbx");
                        self.emit("setg al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::GtE => {
                        self.emit("cmp rax, rbx");
                        self.emit("setge al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::Lt => {
                        self.emit("cmp rax, rbx");
                        self.emit("setl al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::LtE => {
                        self.emit("cmp rax, rbx");
                        self.emit("setle al");
                        self.emit("movzx rax, al");
                    }
                    _ => {}
                }

                self.emit(&format!("mov {}, rax", dst));
            }

            Instruction::Unary { dst, op, value } => {
                let dst = self.frame.addr(dst);

                self.emit(&format!("mov rax, {}", self.lower_value(value)));

                match op {
                    IrOp::Neg => self.emit("neg rax"),
                    IrOp::Pos => {}
                    _ => {}
                }

                self.emit(&format!("mov {}, rax", dst));
            }

            Instruction::Jump(l) => self.emit(&format!("jmp {}", l)),

            Instruction::JumpIfFalse { cond, target } => {
                self.emit(&format!("cmp qword {}, 0", self.lower_value(cond)));
                self.emit(&format!("je {}", target));
            }

            Instruction::Return { value } => {
                self.emit(&format!("mov rax, {}", self.lower_value(value)));
                self.emit("mov rsp, rbp");
                self.emit("pop rbp");
                self.emit("ret");
            }

            Instruction::Extern { fnname } => {
                self.emit(&format!("extern {}", fnname));
            }

            Instruction::Arg { value } => {
                let v = self.lower_value(value);
                if let Some(reg) = self.target.arg_reg(self.ctx.arg_index) {
                    self.emit(&format!("mov {}, {}", reg, v));
                } else {
                    self.emit(&format!("push {}", v));
                }
                self.ctx.arg_index += 1;
            }

            Instruction::Call { name, dest, .. } => {
                self.emit(&format!("call {}", name));
                self.ctx.arg_index = 0;

                // Capture RAX into your destination variable if one exists (e.g., "t1")
                if let Some(d) = dest {
                    let dst_addr = self.frame.addr(d);
                    self.emit(&format!("mov qword {}, rax", dst_addr));
                }
            }

            Instruction::Param { p } => {
                let dst = self.frame.addr(p);
                if let Some(reg) = self.target.arg_reg(self.ctx.param_index) {
                    self.emit(&format!("mov {}, {}", dst, reg));
                }
                self.ctx.param_index += 1;
            }
        }
    }
}

impl Backend for NasmBackend {
    fn new(target: Target) -> Self {
        Self {
            target,
            out: String::new(),
            frame: StackFrame::new(),
            ctx: FunctionCtx {
                arg_index: 0,
                param_index: 0,
            },
        }
    }

    fn emit_program(&mut self, program: &[Instruction]) -> String {
        self.out.clear();
        self.emit("section .text");

        for func in split_functions(program) {
            self.reset();

            self.collect_stack_slots(&func);
            self.frame.finalize();

            for inst in &func {
                self.emit_instruction(inst);
            }
        }

        self.out.clone()
    }
}