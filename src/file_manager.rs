use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Manages file system operations for saving downloaded content
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileManager {
    base_dir: PathBuf,
}

impl FileManager {
    pub fn new(base_dir: &Path) -> Result<Self> {
        let base_dir = base_dir.to_path_buf();
        fs::create_dir_all(&base_dir)
            .with_context(|| format!("Failed to create base directory: {:?}", base_dir))?;

        Ok(Self { base_dir })
    }

    /// Save content to a file at the specified path
    /// The path should be relative to the base directory
    pub fn save_file(&self, relative_path: &Path, content: &[u8]) -> Result<PathBuf> {
        let full_path = self.base_dir.join(relative_path);

        // Create parent directories if they don't exist
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {:?}", parent))?;
        }

        // Write the file
        let mut file = fs::File::create(&full_path)
            .with_context(|| format!("Failed to create file: {:?}", full_path))?;

        file.write_all(content)
            .with_context(|| format!("Failed to write to file: {:?}", full_path))?;

        Ok(full_path)
    }

    /// Check if a file exists at the specified path
    pub fn file_exists(&self, relative_path: &Path) -> bool {
        self.base_dir.join(relative_path).exists()
    }

    /// Read a file's content
    pub fn read_file(&self, relative_path: &Path) -> Result<Vec<u8>> {
        let full_path = self.base_dir.join(relative_path);
        fs::read(&full_path).with_context(|| format!("Failed to read file: {:?}", full_path))
    }

    /// Get the full path for a relative path
    pub fn get_full_path(&self, relative_path: &Path) -> PathBuf {
        self.base_dir.join(relative_path)
    }

    /// Get the base directory
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_new_file_manager() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();
        assert_eq!(file_manager.base_dir, temp_dir.path());
    }

    #[test]
    fn test_save_file() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();

        let content = b"Hello, World!";
        let path = Path::new("test.txt");
        let result = file_manager.save_file(path, content);
        assert!(result.is_ok());

        let saved_path = result.unwrap();
        assert!(saved_path.exists());

        let read_content = fs::read(&saved_path).unwrap();
        assert_eq!(read_content, content);
    }

    #[test]
    fn test_save_file_with_subdirectories() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();

        let content = b"CSS content";
        let path = Path::new("css/style.css");
        let result = file_manager.save_file(path, content);
        assert!(result.is_ok());

        let saved_path = result.unwrap();
        assert!(saved_path.exists());

        // Check that subdirectory was created
        let css_dir = temp_dir.path().join("css");
        assert!(css_dir.exists());
        assert!(css_dir.is_dir());
    }

    #[test]
    fn test_save_file_nested_subdirectories() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();

        let content = b"Nested content";
        let path = Path::new("a/b/c/d/file.txt");
        let result = file_manager.save_file(path, content);
        assert!(result.is_ok());

        let saved_path = result.unwrap();
        assert!(saved_path.exists());

        // Check that all nested directories were created
        let dir_a = temp_dir.path().join("a");
        let dir_b = dir_a.join("b");
        let dir_c = dir_b.join("c");
        let dir_d = dir_c.join("d");

        assert!(dir_a.exists() && dir_a.is_dir());
        assert!(dir_b.exists() && dir_b.is_dir());
        assert!(dir_c.exists() && dir_c.is_dir());
        assert!(dir_d.exists() && dir_d.is_dir());
    }

    #[test]
    fn test_save_file_overwrite() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();

        let content1 = b"First content";
        let content2 = b"Second content";
        let path = Path::new("test.txt");

        // Save first file
        let result1 = file_manager.save_file(path, content1);
        assert!(result1.is_ok());

        // Overwrite with second file
        let result2 = file_manager.save_file(path, content2);
        assert!(result2.is_ok());

        let saved_path = result2.unwrap();
        assert!(saved_path.exists());

        // Check that content was overwritten
        let read_content = fs::read(&saved_path).unwrap();
        assert_eq!(read_content, content2);
    }

    #[test]
    fn test_file_exists() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();

        let path = Path::new("test.txt");
        assert!(!file_manager.file_exists(path));

        // Save a file
        file_manager.save_file(path, b"content").unwrap();
        assert!(file_manager.file_exists(path));
    }

    #[test]
    fn test_read_file() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();

        let content = b"Test content";
        let path = Path::new("test.txt");

        // Save file first
        file_manager.save_file(path, content).unwrap();

        // Read it back
        let read_content = file_manager.read_file(path).unwrap();
        assert_eq!(read_content, content);
    }

    #[test]
    fn test_read_file_not_exists() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();

        let path = Path::new("nonexistent.txt");
        let result = file_manager.read_file(path);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_full_path() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();

        let relative = Path::new("subdir/file.txt");
        let full = file_manager.get_full_path(relative);

        assert_eq!(full, temp_dir.path().join("subdir/file.txt"));
    }

    #[test]
    fn test_file_manager_debug() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();
        let debug_str = format!("{:?}", file_manager);
        assert!(debug_str.contains("FileManager"));
        assert!(debug_str.contains(temp_dir.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn test_file_manager_clone() {
        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();
        let cloned = file_manager.clone();

        assert_eq!(file_manager.base_dir, cloned.base_dir);
    }

    #[test]
    fn test_file_manager_partial_eq() {
        let temp_dir = tempdir().unwrap();
        let file_manager1 = FileManager::new(temp_dir.path()).unwrap();
        let file_manager2 = FileManager::new(temp_dir.path()).unwrap();

        assert_eq!(file_manager1, file_manager2);
    }

    #[test]
    fn test_file_manager_hash() {
        use std::collections::HashMap;

        let temp_dir = tempdir().unwrap();
        let file_manager = FileManager::new(temp_dir.path()).unwrap();

        let mut map = HashMap::new();
        map.insert(file_manager.clone(), "value");

        assert_eq!(map.get(&file_manager), Some(&"value"));
    }
}
