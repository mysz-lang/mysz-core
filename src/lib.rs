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
    output_json: bool,
) -> Result<(), String> {
    compiler::compile_root_file(input_path, output_filename, search_paths, output_json)
}

pub fn check_file<P: AsRef<Path>>(
    input_path: P,
    search_paths: &[PathBuf],
    output_json: bool,
) -> Result<(), String> {
    compiler::check_root_file(input_path, search_paths, output_json)
}
