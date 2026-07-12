pub mod backend;
pub mod compiler;
pub mod ir;
pub mod lexing;
pub mod parse;
pub mod semantics;
pub mod utils;

use std::path::{Path, PathBuf};

pub fn compile_file<P: AsRef<Path>>(
    input_path: P,
    output_filename: &str,
    search_paths: &[PathBuf],
) -> Result<(), String> {
    compiler::compile_root_file(input_path, output_filename, search_paths)
}
