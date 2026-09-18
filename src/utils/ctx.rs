use std::{
    path::{Path, PathBuf},
    process::exit,
};

pub struct CompilerCtx<'a, P: AsRef<Path>> {
    pub input_path: P,
    pub search_paths: &'a [PathBuf],
    pub output_json: bool,
    pub debug: bool,
}

impl<'a, P: AsRef<Path>> CompilerCtx<'a, P> {
    pub fn new(input_path: P, search_paths: &'a [PathBuf], output_json: bool, debug: bool) -> Self {
        if output_json && debug {
            eprintln!(
                "Cannot have both json output and debugging enabled. They are inherintly incompatible as of now."
            );
            exit(-1);
        }

        Self {
            input_path,
            search_paths,
            output_json,
            debug,
        }
    }
}
