use std::path::{Path, PathBuf};

pub struct CompilerCtx<'a, P: AsRef<Path>> {
    pub input_path: P,
    pub search_paths: &'a [PathBuf],
    pub output_json: bool,
}

impl<'a, P: AsRef<Path>> CompilerCtx<'a, P> {
    pub fn new(input_path: P, search_paths: &'a [PathBuf], output_json: bool) -> Self {
        Self {
            input_path,
            search_paths,
            output_json,
        }
    }
}
