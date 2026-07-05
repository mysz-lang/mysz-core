![Mysz Icon](https://raw.githubusercontent.com/mysz-lang/.github/main/images/mysz_logo_1x.jpg)

# mysz-core
A mouse-loving mouse-based mouse-enthusiastic programming language project.

This is the core repository, which holds the rust source code for the Lexer, Parser, Semantic analyser, Intermediate Code Generator, Utils for core, and assembly code generator of Mysz.

# Support

| OS | architecture | Supported |
|---|---|---|
| Linux (nasm) | x86_64 | Supported |
| Linux (gas) | x86_64 | Planned (Not supported) |
| Windows (masm) | x86_64 | Planned (Not supported) |
| Windows (gas) | x86_64 | Planned (Not supported) |
| MacOs | ARM_64 | Not Planned |

## Embedding the Core Engine

`mysz-core` is designed as a standalone library crate that can be driven by external CLI tools or environments. Add it to your project's `Cargo.toml`:

```toml
[dependencies]
mysz-core = { git = "[https://github.com/mysz-lang/mysz-core.git](https://github.com/mysz-lang/mysz-core.git)", branch = "main" }
```

### Basic Compilation Example

```rust
use mysz_core::compile_source;

fn main() {
    let source = "extern fn print_int(a: int); fn main() { var x = 60; print_int(x); };";
    
    match compile_source(source, "x86_64_linux", "output.asm") {
        Ok(()) => println!("Successfully compiled to output.asm"),
        Err(e) => eprintln!("Compiler Error:\n{}", e),
    }
}
```
