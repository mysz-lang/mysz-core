use std::path::{Path, PathBuf};

pub enum CompilerTarget {
    Cranelift,
    Llvm,
}

pub struct CompilerCtx<'a, P: AsRef<Path>> {
    pub input_path: P,
    pub search_paths: &'a [PathBuf],
    pub output_json: bool,
    pub target: CompilerTarget,
}

impl<'a, P: AsRef<Path>> CompilerCtx<'a, P> {
    pub fn new(
        input_path: P,
        search_paths: &'a [PathBuf],
        output_json: bool,
        target: CompilerTarget,
    ) -> Self {
        Self {
            input_path,
            search_paths,
            output_json,
            target,
        }
    }
}