![Mysz Icon](https://raw.githubusercontent.com/mysz-lang/.github/main/images/mysz_logo_1x.jpg)

# mysz-core
A mouse-loving, mouse-based, mouse-enthusiastic programming language project.

This is the core repository, which holds the rust source code for the Lexer, Parser, Semantic analyser, Intermediate Code Generator, Utils for core, and assembly code generator of Mysz.

# Support

| OS | architecture | Supported |
|---|---|---|
| Linux (generic) | x86_64 | Supported (via cranelift) |
| Windows msvc | x86_64 | Planned |
| MacOs | ARM_64 | Unsupported |

## Embedding the Core Engine

`mysz-core` is designed as a standalone library crate that can be driven by external CLI tools or environments. Add it to your project's `Cargo.toml`:

```toml
[dependencies]
mysz-core = { git = "https://github.com/mysz-lang/mysz-core.git", branch = "main" }
```

### Using crates.io

`mysz-core` is now on [crates.io](https://crates.io/crates/mysz-core), making adding it to your project's `Cargo.toml` is easy:

```toml
[dependencies]
mysz-core = "0.2.0"
```

`mysz-core` will output object files, it is the responsibility of the embedding environment, or developer to link it and output a binary or library.

### Basic Compilation Example

```rust
use mysz_core::compile_source;

fn main() {
    let source = "extern fn print_int(a: int); fn main() { var x = 60; print_int(x); };";
    
    match compile_source(source, "output.o") {
        Ok(()) => println!("Successfully compiled to output.o"),
        Err(e) => eprintln!("Compiler Error:\n{}", e),
    }
}
```

### About String concatenation

The + operator can be used to concatenate strings:

```
"Hello, " + "world!"
```

To enable string concatenation, the host application or runtime must provide a function named str_concat that returns the concatenated string.

Whenever the compiler encounters a string concatenation expression, it generates a call to str_concat. It is the responsibility of the embedding environment or standard library to provide an implementation of this function.
