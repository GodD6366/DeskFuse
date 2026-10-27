using System.Text.Json;

namespace DeskFuse.Windows;

/// Typed host ports keep clipboard/OLE operations in the interactive process. The
/// service only authenticates the bounded request and returns an operation
/// receipt; it never opens the user's clipboard from Session 0.
internal interface ClipboardCapturePort
{
    Task<ClipboardContent> CaptureAsync(long sessionEpoch, long sequence, CancellationToken token = default);
}

internal interface ClipboardApplyPort
{
    Task<ClipboardContent> ApplyAsync(ClipboardContent content, long expectedRevision, long sessionEpoch, long sequence, CancellationToken token = default);
}

internal interface WorkflowExecutionPort
{
    Task<ClipboardContent> ExecuteAsync(string workflowId, bool paste, IntPtr target, long sessionEpoch, long sequence,
        CancellationToken token = default, ClipboardContent? source = null);
    /// <summary>Run without touching the local clipboard; used before sharing the result.</summary>
    Task<ClipboardContent> ExecuteToContentAsync(string workflowId, ClipboardContent source, long sessionEpoch,
        long sequence, CancellationToken token = default);
}

internal interface FileDataObjectPort
{
    Task<string> CreateFileDataObjectAsync(IReadOnlyList<string> paths, long sessionEpoch, long sequence, string? targetPeerId = null);
}

internal static class PlatformBoundary
{
    internal static object CreateFileDataObject(IReadOnlyList<string> paths, string? targetPeerId = null) =>
        new { CreateFileDataObject = new { paths, target_peer_id = targetPeerId } };

    internal static object ReadClipboardHost(ClipboardContent content, long revision)
    {
        var representations = content.Representations ?? [new ClipboardRepresentation(content.Mime, content.Data)];
        var primary = representations.FirstOrDefault() ?? new ClipboardRepresentation(content.Mime, content.Data);
        return new
        {
            ReadClipboardHost = new
            {
                content = new
                {
                    revision,
                    types = representations.Select(item => item.Mime).ToArray(),
                    data = new
                    {
                        mime = primary.Mime,
                        bytes = primary.Data.Select(value => (int)value).ToArray(),
                        file_urls = content.Files ?? Array.Empty<string>(),
                        items = Array.Empty<object>()
                    },
                    source = "User",
                    captured_at_ms = (ulong)Environment.TickCount64
                }
            }
        };
    }

    internal static object WriteClipboardHost(ClipboardContent content, long expectedRevision)
    {
        var representations = content.Representations ?? [new ClipboardRepresentation(content.Mime, content.Data)];
        var primary = representations.FirstOrDefault() ?? new ClipboardRepresentation(content.Mime, content.Data);
        return new
        {
            WriteClipboardHost = new
            {
                content = new
                {
                    data = new
                    {
                        mime = primary.Mime,
                        bytes = primary.Data.Select(value => (int)value).ToArray(),
                        file_urls = content.Files ?? Array.Empty<string>(),
                        items = Array.Empty<object>()
                    },
                    remote_origin_marker = (string?)null
                },
                expected_revision = expectedRevision
            }
        };
    }

    internal static object ExecuteWorkflowHost(string workflowId, ClipboardContent input)
    {
        // Keep every captured representation so image workflows receive the
        // native PNG and text workflows still see plain text. The service
        // chooses the payload from the declared input type and steps.
        var representations = input.Representations ?? [new ClipboardRepresentation(input.Mime, input.Data)];
        var items = new List<object>();
        if (input.Files is { Count: > 0 })
        {
            items.Add(new
            {
                mime = ClipboardContent.LocalFilesMime,
                bytes = System.Text.Encoding.UTF8.GetBytes(JsonSerializer.Serialize(input.Files)),
            });
        }
        else
        {
            foreach (var representation in representations)
                items.Add(new { mime = representation.Mime, bytes = representation.Data.Select(value => (int)value).ToArray() });
        }
        return new
        {
            ExecuteWorkflowHost = new
            {
                workflow_id = workflowId,
                input = items,
            }
        };
    }

    internal static ClipboardContent ReadContent(JsonElement eventValue)
    {
        if (!eventValue.TryGetProperty("ClipboardCaptured", out var captured))
            throw ReceiptError(eventValue, "服务未返回 typed 剪贴板读取回执。");
        if (!captured.TryGetProperty("data", out var data))
            throw new IOException("服务返回了不完整的剪贴板读取回执。");
        return Content(data);
    }

    internal static ClipboardContent WorkflowContent(JsonElement eventValue)
    {
        if (!eventValue.TryGetProperty("WorkflowCompleted", out var completed))
            throw ReceiptError(eventValue, "服务未返回 typed 工作流完成回执。");
        if (!completed.TryGetProperty("content", out var content) || content.ValueKind == JsonValueKind.Null)
            throw new IOException("工作流未返回可写入剪贴板的结果。");
        return Content(content);
    }

    internal static bool TryGetFileDataObjectCreated(JsonElement reply, out string id, out string error)
    {
        id = "";
        error = "";
        if (!TryEvent(reply, out var value))
        {
            error = "服务未返回 typed 文件数据对象回执。";
            return false;
        }
        if (value.TryGetProperty("FileDataObjectCreated", out var created))
        {
            id = created.String("id");
            if (id.Length > 0) return true;
            error = "服务返回了空的文件数据对象标识。";
            return false;
        }
        error = Error(value, "文件数据对象创建失败。");
        return false;
    }

    internal static JsonElement Event(JsonElement reply)
    {
        if (TryEvent(reply, out var value)) return value;
        throw new IOException("服务未返回 typed platform 回执。");
    }

    private static bool TryEvent(JsonElement reply, out JsonElement value)
    {
        value = default;
        return reply.TryGetProperty("Platform", out var platform)
            && platform.TryGetProperty("event", out value)
            && value.ValueKind == JsonValueKind.Object;
    }

    private static ClipboardContent Content(JsonElement value)
    {
        string mime = value.String("mime", "text/plain");
        byte[] bytes = value.TryGetProperty("bytes", out var raw) && raw.ValueKind == JsonValueKind.Array
            ? raw.EnumerateArray().Select(item => item.GetByte()).ToArray()
            : Array.Empty<byte>();
        var files = value.Array("file_urls").Where(item => item.ValueKind == JsonValueKind.String)
            .Select(item => item.GetString() ?? "").ToArray();
        return new ClipboardContent(mime, bytes, files.Length == 0 ? null : files);
    }

    private static IOException ReceiptError(JsonElement eventValue, string fallback) =>
        new(Error(eventValue, fallback));

    private static string Error(JsonElement eventValue, string fallback) =>
        eventValue.TryGetProperty("Failed", out var failed) ? failed.String("message", fallback) : fallback;
}

internal sealed class WindowsWorkflowExecutionPort(ServiceClient service, DesktopClipboardController clipboard) : WorkflowExecutionPort
{
    public async Task<ClipboardContent> ExecuteAsync(string workflowId, bool paste, IntPtr target, long sessionEpoch, long requestSequence, CancellationToken token = default, ClipboardContent? source = null)
    {
        var input = source ?? await clipboard.CaptureTypedAsync(sessionEpoch, requestSequence, token);
        if (input.Files is { Count: > 0 }) throw new InvalidOperationException("Windows 工作流暂不支持文件输入。");
        var reply = await service.SendPlatformAsync(
            sessionEpoch,
            requestSequence + 1,
            PlatformBoundary.ExecuteWorkflowHost(workflowId, input),
            token);
        var output = PlatformBoundary.WorkflowContent(PlatformBoundary.Event(reply));
        long expectedRevision = clipboard.NativeRevision;
        var applied = await clipboard.ApplyTypedAsync(
            output,
            expectedRevision,
            sessionEpoch,
            requestSequence + 2,
            token);
        if (paste)
        {
            if (target == IntPtr.Zero)
                throw new InvalidOperationException("没有可用的目标应用，结果已复制到剪贴板。");
            // Reuse the quick-panel boundary: wait for the shortcut modifiers to
            // be released, verify the clipboard still holds our result, then paste.
            await ClipboardPanelNative.PasteWhenReadyAsync(target, clipboard.Native.SequenceNumber, clipboard.Native, token);
        }
        return applied;
    }

    public async Task<ClipboardContent> ExecuteToContentAsync(string workflowId, ClipboardContent source,
        long sessionEpoch, long requestSequence, CancellationToken token = default)
    {
        if (source.Files is { Count: > 0 }) throw new InvalidOperationException("Windows 工作流暂不支持文件输入。");
        var reply = await service.SendPlatformAsync(
            sessionEpoch,
            requestSequence,
            PlatformBoundary.ExecuteWorkflowHost(workflowId, source),
            token);
        return PlatformBoundary.WorkflowContent(PlatformBoundary.Event(reply));
    }
}
