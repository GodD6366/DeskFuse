#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    async fn clipboard_capture(&self) -> Result<ClipboardContent>;
    async fn clipboard_apply(&self, content: ClipboardContent) -> Result<ApplyResult>;
    async fn authorize_file_read(&self, paths: &[PathBuf]) -> Result<Vec<FileHandle>>;
    async fn execute_input(&self, event: InputEvent) -> Result<()>;
}
