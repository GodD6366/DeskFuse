using System.Text.Json;

namespace UniDesk.Windows;

/// Typed host ports keep WPF/OLE operations in the interactive process. The
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
    Task<ClipboardContent> ExecuteAsync(string workflowId, long sessionEpoch, long sequence, CancellationToken token = default);
}

internal interface FileDataObjectPort
{
    Task<string> CreateFileDataObjectAsync(IReadOnlyList<string> paths, long sessionEpoch, long sequence);
}

internal static class PlatformBoundary
{
    internal static object CreateFileDataObject(IReadOnlyList<string> paths) =>
        new { CreateFileDataObject = new { paths } };

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
        var representation = input.Representations?.FirstOrDefault(item => item.Mime == "text/plain")
            ?? new ClipboardRepresentation(input.Mime, input.Data);
        return new
        {
            ExecuteWorkflowHost = new
            {
                workflow_id = workflowId,
                input = new[] { new { mime = representation.Mime, bytes = representation.Data.Select(value => (int)value).ToArray() } }
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
    public async Task<ClipboardContent> ExecuteAsync(string workflowId, long sessionEpoch, long requestSequence, CancellationToken token = default)
    {
        var input = await clipboard.CaptureTypedAsync(sessionEpoch, requestSequence, token);
        if (input.Files is { Count: > 0 }) throw new InvalidOperationException("工作流当前只接受文字剪贴板输入。");
        var reply = await service.SendPlatformAsync(
            sessionEpoch,
            requestSequence + 1,
            PlatformBoundary.ExecuteWorkflowHost(workflowId, input),
            token);
        var output = PlatformBoundary.WorkflowContent(PlatformBoundary.Event(reply));
        long expectedRevision = clipboard.NativeRevision;
        return await clipboard.ApplyTypedAsync(
            output,
            expectedRevision,
            sessionEpoch,
            requestSequence + 2,
            token);
    }
}
