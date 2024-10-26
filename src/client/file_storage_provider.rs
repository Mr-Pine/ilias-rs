use std::{fs::File, io::BufReader, path::PathBuf};

use stream_download::storage::StorageProvider;

#[derive(Debug, Clone)]
pub struct FileStorageProvider(PathBuf);

impl FileStorageProvider {
    pub fn new(path: PathBuf) -> FileStorageProvider {
        FileStorageProvider(path)
    }
}

impl StorageProvider for FileStorageProvider {
    type Reader = BufReader<File>;

    type Writer = File;

    fn into_reader_writer(
        self,
        _content_length: Option<u64>,
    ) -> std::io::Result<(Self::Reader, Self::Writer)> {
        let mut options = File::options();
        options.write(true);
        options.create(true);

        let read_file = options.open(&self.0)?;
        let reader = BufReader::new(read_file);
        let write_file = options.open(&self.0)?;

        Ok((reader, write_file))
    }
}
