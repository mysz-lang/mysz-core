pub mod athelp;
pub mod backend;
pub mod compiler;
pub mod ir;
pub mod lex;
pub mod parse;
pub mod semantics;
pub mod utils;

use std::path::Path;

use crate::utils::ats::{ATEntry, ATInfo};
use crate::utils::ctx::CompilerCtx;

pub fn compile_at_graph<'a, P: AsRef<Path>>(
    ctx: &CompilerCtx<'a, P>,
    ats: &[ATInfo],
    entry: &ATEntry,
    output_filename: &str,
) -> Result<(), String> {
    compiler::compile_at_graph(ctx, ats, entry, output_filename)
}
pub fn check_at_graph<'a, P: AsRef<Path>>(
    ctx: &CompilerCtx<'a, P>,
    ats: &[ATInfo],
    entry: &ATEntry,
) -> Result<(), String> {
    compiler::check_at(ctx, ats, entry)
}

pub fn compile_file<'a, P: AsRef<Path>>(
    ctx: &CompilerCtx<'a, P>,
    ats: &[ATInfo],
    file_path: P,
    output_filename: &str,
) -> Result<(), String> {
    let file_path = file_path
        .as_ref()
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize path: {}", e))?;

    let entry_idx = ats
        .iter()
        .position(|at| at.entry_file == file_path || at.files.iter().any(|f| f.path == file_path));

    let entry = match entry_idx {
        Some(idx) => ATEntry {
            info: idx,
            is_current: true,
        },
        None => {
            return Err(format!(
                "File '{}' does not belong to any known AT",
                file_path.display()
            ));
        }
    };

    compiler::compile_at_graph(ctx, ats, &entry, output_filename)
}

pub fn check_file<'a, P: AsRef<Path>>(
    ctx: &CompilerCtx<'a, P>,
    ats: &[ATInfo],
    file_path: P,
) -> Result<(), String> {
    let file_path = file_path
        .as_ref()
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize path: {}", e))?;

    let entry_idx = ats
        .iter()
        .position(|at| at.entry_file == file_path || at.files.iter().any(|f| f.path == file_path));

    let entry = match entry_idx {
        Some(idx) => ATEntry {
            info: idx,
            is_current: true,
        },
        None => {
            return Err(format!(
                "File '{}' does not belong to any known AT",
                file_path.display()
            ));
        }
    };

    compiler::check_at(ctx, ats, &entry)
}
