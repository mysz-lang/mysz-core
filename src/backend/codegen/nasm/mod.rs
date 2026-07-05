use std::collections::HashMap;

use crate::ir::tac::{Instruction, IrOp, Value};
use crate::backend::codegen::{Target, Backend};
use crate::parse::parsing::Type;

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
        self.slots.insert(name.to_string(), self.next_offset + 8);
        
        self.next_offset += 8;
    }

    fn finalize(&mut self) {
        let total_bytes_used = self.next_offset;
        
        self.size = (total_bytes_used + 15) & !15;
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
    
    rodata: HashMap<String, String>,
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
            
            Value::Str(s) => match self.rodata.get(s) {
                Some(label) => label.clone(),
                None => panic!("String literal was never collected in pre-pass: {}", s),
            },
        }
    }

    fn collect_value(&mut self, v: &Value) {
        match v {
            Value::Temp(t) | Value::Var(t) => self.frame.alloc(t),
            Value::Str(s) => {
                if !self.rodata.contains_key(s) {
                    let id = self.rodata.len();
                    self.rodata.insert(s.clone(), format!("msg{}", id));
                }
            }
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

                Instruction::Load { dst, ptr, .. } => {
                    self.frame.alloc(dst);
                    self.collect_value(ptr);
                }
                
                Instruction::Store { ptr, source } => {
                    self.collect_value(ptr);
                    self.collect_value(source);
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
                self.emit(&format!("global {}", name));
                self.emit(&format!("{}:", name));
                self.emit_prologue();
            }

            Instruction::Store { ptr, source } => {
                let ptr_loc = self.lower_value(ptr);
                
                self.emit(&format!("mov rax, {}", ptr_loc));

                let src_val = self.lower_value(source);

                match source {
                    Value::Const(_) | Value::Bool(_) | Value::Void => {
                        self.emit(&format!("mov qword [rax], {}", src_val));
                    }
                    Value::Temp(_) | Value::Var(_) => {
                        self.emit(&format!("mov r10, {}", src_val));
                        self.emit("mov [rax], r10");
                    }
                    Value::Str(_) => {
                        self.emit(&format!("lea r10, [rel {}]", src_val));
                        self.emit("mov [rax], r10");
                    }
                }
            }

            Instruction::Label(l) => self.emit(&format!("{}:", l)),

            Instruction::Assign { dst, src } => {
                let dst_addr = self.frame.addr(dst);
                let src_val = self.lower_value(src);
                
                if dst_addr.starts_with('[') && src_val.starts_with('[') {
                    self.emit(&format!("mov rax, {}", src_val));
                    self.emit(&format!("mov qword {}, rax", dst_addr));
                    } else if !src_val.starts_with('[') && !src_val.chars().all(|c| c.is_ascii_digit() || c == '-') {
                        self.emit(&format!("lea rax, [rel {}]", src_val));
                        self.emit(&format!("mov qword {}, rax", dst_addr));
                    } else {
                    self.emit(&format!("mov qword {}, {}", dst_addr, src_val));
                }
            }

            Instruction::Binary { dst, op, lhs, rhs } => {
                let dst = self.frame.addr(dst);

                self.emit(&format!("mov rax, {}", self.lower_value(lhs)));
                self.emit(&format!("mov r10, {}", self.lower_value(rhs)));

                match op {
                    IrOp::Add => self.emit("add rax, r10"),
                    IrOp::Sub => self.emit("sub rax, r10"),
                    IrOp::Mul => self.emit("imul rax, r10"),
                    IrOp::Div => {
                        self.emit("cqo");
                        self.emit("idiv r10");
                    }
                    IrOp::Eq => {
                        self.emit("cmp rax, r10");
                        self.emit("sete al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::NEq => {
                        self.emit("cmp rax, r10");
                        self.emit("setne al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::Gt => {
                        self.emit("cmp rax, r10");
                        self.emit("setg al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::GtE => {
                        self.emit("cmp rax, r10");
                        self.emit("setge al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::Lt => {
                        self.emit("cmp rax, r10");
                        self.emit("setl al");
                        self.emit("movzx rax, al");
                    }
                    IrOp::LtE => {
                        self.emit("cmp rax, r10");
                        self.emit("setle al");
                        self.emit("movzx rax, al");
                    }
                    unsup => self.emit(&format!("; unsupported IrOp used: {:?}", unsup)),                }

                self.emit(&format!("mov {}, rax", dst));
            }

            Instruction::Unary { dst, op, value } => {
                let dst_slot = self.frame.addr(dst);

                match op {
                    IrOp::Ref => {
                        let var_loc = self.lower_value(value);
                        
                        let raw_address = var_loc.trim_matches(|c| c == '[' || c == ']');
                        
                        self.emit(&format!("lea rax, [{}]", raw_address));
                    }
                    IrOp::Neg => {
                        self.emit(&format!("mov rax, {}", self.lower_value(value)));
                        self.emit("neg rax");
                    }
                    IrOp::Pos => {
                        self.emit(&format!("mov rax, {}", self.lower_value(value)));
                    }

                    _ => {}
                }

                self.emit(&format!("mov {}, rax", dst_slot));
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
                
                let source_expr = if !v.starts_with('[') && !v.chars().all(|c| c.is_ascii_digit() || c == '-') {
                    self.emit(&format!("lea rax, [rel {}]", v));
                    "rax".to_string()
                } else {
                    v
                };

                if let Some(reg) = self.target.arg_reg(self.ctx.arg_index) {
                    self.emit(&format!("mov {}, {}", reg, source_expr));
                } else {
                    self.emit(&format!("push {}", source_expr));
                }
                self.ctx.arg_index += 1;
            }

            Instruction::Call { name, dest, .. } => {
                self.emit(&format!("call {}", name));
                self.ctx.arg_index = 0;

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


            Instruction::Load { dst, ptr, ty } => {
                let ptr_loc = self.lower_value(ptr);
                let dst_slot = self.frame.addr(dst);

                self.emit(&format!("mov rax, {}", ptr_loc));
                
                if !matches!(ty, Type::Array { .. }) {
                    self.emit("mov rax, [rax]");
                }
                
                self.emit(&format!("mov {}, rax", dst_slot));
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
            rodata: HashMap::new(),
        }
    }

    fn emit_program(&mut self, program: &[Instruction]) -> String {
        self.out.clear();
        self.rodata.clear();
        for inst in program {
            match inst {
                Instruction::Assign { src, .. } => self.collect_value(src),
                Instruction::Binary { lhs, rhs, .. } => {
                    self.collect_value(lhs);
                    self.collect_value(rhs);
                }
                Instruction::Unary { value, .. } => self.collect_value(value),
                Instruction::Arg { value } => self.collect_value(value),
                Instruction::Return { value } => self.collect_value(value),
                Instruction::Store { ptr, source } => self.collect_value(val);
                _ => {}
            }
        }

        if !self.rodata.is_empty() {
            self.emit("section .rodata");

            let mut items: Vec<(String, String)> = self.rodata
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            items.sort_by(|a, b| a.1.cmp(&b.1));

            for (raw_str, label) in items {
                self.emit(&format!("    {}: db \"{}\", 0", label, raw_str));
            }
            self.emit("");
        }

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
