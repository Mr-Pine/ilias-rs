use std::path::PathBuf;

/// A file on the local file system and the name it should have when uploaded
#[derive(Debug, Clone)]
pub struct NamedLocalFile {
    pub name: String,
    pub path: PathBuf,
}
