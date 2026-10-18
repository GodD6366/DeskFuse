use crate::executor::WorkflowOutput;
use deskfuse_collaboration::{CollaborationCommand, CollaborationCore, ContentHandle};
use deskfuse_platform::ports::ContentData;

/// Share workflow output through the collaboration clipboard API when requested.
pub async fn share_output(
    core: &CollaborationCore,
    output: &WorkflowOutput,
) -> deskfuse_collaboration::Result<()> {
    if !output.share_with_devices {
        return Ok(());
    }
    let handle = ContentHandle::from_data(&ContentData::text(output.text.clone()));
    core.submit(CollaborationCommand::PublishClipboard { content: handle })
        .await
}
