use crate::utils::ats::{ATDependency, ATFile, ATInfo};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct ATBuilder {
    name: Option<String>,
    version: Option<String>,
    root_dir: Option<PathBuf>,
    entry_file: Option<PathBuf>,
    files: Vec<ATFile>,
    dependencies: Vec<ATDependency>,
}

impl ATBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the AT name (required).
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the version (optional).
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set the root directory (defaults to current working directory).
    pub fn root_dir(mut self, root: impl AsRef<Path>) -> Self {
        self.root_dir = Some(root.as_ref().to_path_buf());
        self
    }

    /// Set the entry file (if not set, will try to detect).
    pub fn entry_file(mut self, entry: impl AsRef<Path>) -> Self {
        self.entry_file = Some(entry.as_ref().to_path_buf());
        self
    }

    /// Add a single file to the AT (manually).
    pub fn add_file(mut self, path: impl AsRef<Path>, module_path: Vec<String>) -> Self {
        self.files.push(ATFile {
            path: path.as_ref().to_path_buf(),
            module_path,
        });
        self
    }

    /// Add a dependency AT.
    pub fn add_dependency(mut self, name: impl Into<String>, version_req: Option<String>) -> Self {
        self.dependencies.push(ATDependency {
            name: name.into(),
            version_req,
        });
        self
    }

    /// Automatically discover all .mysz files in the root directory.
    /// Files are mapped to module paths based on their path relative to root.
    /// The entry file will be set to the one at root (if exists) or the first file.
    pub fn discover_files(mut self) -> Result<Self, String> {
        let root = self
            .root_dir
            .clone()
            .ok_or("Root directory must be set before discovery")?;
        let mut files = Vec::new();

        fn visit_dir(
            dir: &Path,
            _root: &Path,
            prefix: Vec<String>,
            files: &mut Vec<ATFile>,
        ) -> Result<(), String> {
            let entries = fs::read_dir(dir)
                .map_err(|e| format!("Failed to read dir {}: {}", dir.display(), e))?;
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                if path.is_dir() {
                    let mut new_prefix = prefix.clone();
                    new_prefix.push(path.file_stem().unwrap().to_string_lossy().to_string());
                    visit_dir(&path, _root, new_prefix, files)?;
                } else if path.extension().map(|e| e == "mysz").unwrap_or(false) {
                    let module_path = prefix.clone();
                    let module_name = path.file_stem().unwrap().to_string_lossy().to_string();
                    let mut full_module_path = module_path;
                    full_module_path.push(module_name);
                    files.push(ATFile {
                        path,
                        module_path: full_module_path,
                    });
                }
            }
            Ok(())
        }

        visit_dir(&root, &root, Vec::new(), &mut files)?;

        if self.entry_file.is_none() {
            let root_files: Vec<_> = files.iter().filter(|f| f.module_path.len() == 1).collect();
            if let Some(main) = root_files.iter().find(|f| f.module_path[0] == "main") {
                self.entry_file = Some(main.path.clone());
            } else if let Some(first) = root_files.first() {
                self.entry_file = Some(first.path.clone());
            } else {
                return Err("No .mysz files found in root directory".to_string());
            }
        }

        self.files = files;
        Ok(self)
    }

    /// Build the final ATInfo.
    pub fn build(self) -> Result<ATInfo, String> {
        let name = self.name.ok_or("AT name is required")?;
        let root_dir = self.root_dir.ok_or("Root directory is required")?;
        let entry_file = self
            .entry_file
            .ok_or("Entry file is required (and not detected)")?;

        if !self.files.iter().any(|f| f.path == entry_file) {
            return Err(format!(
                "Entry file {:?} is not in the file list",
                entry_file
            ));
        }

        Ok(ATInfo {
            name,
            version: self.version,
            root_dir,
            entry_file,
            files: self.files,
            dependencies: self.dependencies,

            // The rest are empty, to be filled later by the compiler.
            struct_defs: std::collections::HashMap::new(),
            struct_blueprints: std::collections::HashMap::new(),
            enum_defs: std::collections::HashMap::new(),
            fn_blueprints: std::collections::HashMap::new(),
            analyser_constants: std::collections::HashMap::new(),
            exported: HashSet::new(),
        })
    }
}

/// Convenience function to build an AT from a directory with automatic discovery.
pub fn at_from_directory(name: &str, root_dir: impl AsRef<Path>) -> Result<ATInfo, String> {
    ATBuilder::new()
        .name(name)
        .root_dir(root_dir)
        .discover_files()?
        .build()
}
