using System.Buffers.Binary;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Windows.Threading;
using Microsoft.Win32.SafeHandles;

namespace UniDesk.Windows;

// Never grants FILE_CREATE_PIPE_INSTANCE via GENERIC_WRITE. The authenticated
// LocalSystem service is the only network/identity owner, even while offline.
public sealed class ServiceClient(Dispatcher dispatcher) : IDisposable
{
    private const int MaxFrame = 24 * 1024 * 1024;
    private const string Pipe = @"\\.\pipe\com.unidesk.app.desktop.v2";
    private readonly CancellationTokenSource stop = new();
    private readonly SemaphoreSlim requests = new(1, 1);
    private long nextRequest;
    private Task? poll;
    public bool IsConnected { get; private set; }
    public JsonElement? LatestSnapshot { get; private set; }
    public event Action<JsonElement>? SnapshotChanged;
    public event Action<JsonElement>? ClipboardReceived;
    public event Action? PollStarting;
    private long? sessionEpoch;
    private long clipboardCursor;
    private static long lastStartAttempt;
    private static string lastStartMessage = "后台组件暂未就绪，稍后自动重试。";
    public event Action<bool, string>? ConnectionChanged;
    public static object Command(string name, object args) => new Dictionary<string, object> { [name] = args };
    /// <summary>Send a typed platform request without changing the legacy v2
    /// command shapes used by older desktop clients.</summary>
    public Task<JsonElement> SendPlatformAsync(long sessionEpoch, long sequence, object command, CancellationToken token = default) =>
        SendAsync(Command("Platform", new { session_epoch = sessionEpoch, sequence, command }), token);
    public void Start() => poll ??= Task.Run(PollAsync);
    private async Task PollAsync()
    {
        while (!stop.IsCancellationRequested)
        {
            try
            {
                bool initial = !sessionEpoch.HasValue;
                if (initial) await Task.Run(() => EnsureRunning(stop.Token), stop.Token);
                if (!initial) await dispatcher.InvokeAsync(() => PollStarting?.Invoke());
                var reply = await SendAsync(initial ? "Snapshot" : Command("Poll", new { session_epoch = sessionEpoch!.Value, after_sequence = clipboardCursor }), stop.Token);
                JsonElement snapshot;
                JsonElement? incoming = null;
                if (initial && reply.TryGetProperty("Snapshot", out snapshot)) { sessionEpoch = snapshot.Number("session_epoch"); clipboardCursor = 0; }
                else if (!initial && reply.TryGetProperty("Poll", out var result))
                {
                    snapshot = result.GetProperty("snapshot");
                    clipboardCursor = result.Number("clipboard_sequence");
                    if (result.TryGetProperty("clipboard", out var item) && item.ValueKind == JsonValueKind.Object) incoming = item.Clone();
                }
                else throw new IOException("服务未提供桌面状态，请更新服务和客户端到同一版本。");
                var clone = snapshot.Clone();
                await dispatcher.InvokeAsync(() => { LatestSnapshot = clone; IsConnected = true; ConnectionChanged?.Invoke(true, "服务已连接"); SnapshotChanged?.Invoke(clone); if (incoming.HasValue) ClipboardReceived?.Invoke(incoming.Value); });
            }
            catch (OperationCanceledException) when (stop.IsCancellationRequested) { break; }
            catch (Exception ex)
            {
                string message = ex is EndOfStreamException ? "后台连接中断，正在自动重连；本机历史仍可使用。" : ex is TimeoutException or OperationCanceledException ? "后台组件尚未就绪，正在自动重试。" : ex is IOException ? ex.Message : "后台连接暂时不可用，正在自动重试。";
                await dispatcher.InvokeAsync(() => { IsConnected = false; LatestSnapshot = null; sessionEpoch = null; clipboardCursor = 0; ConnectionChanged?.Invoke(false, message); });
            }
            try { await Task.Delay(TimeSpan.FromSeconds(1), stop.Token); }
            catch (OperationCanceledException) { break; }
        }
    }
    public async Task<JsonElement> SendAsync(object command, CancellationToken token = default)
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(token, stop.Token);
        timeout.CancelAfter(TimeSpan.FromSeconds(12));
        await requests.WaitAsync(timeout.Token).ConfigureAwait(false);
        try
        {
            long id = Interlocked.Increment(ref nextRequest);
            var body = JsonSerializer.SerializeToUtf8Bytes(new { version = 2, request_id = id, command });
            if (body.Length is 0 or > MaxFrame) throw new IOException("请求内容超过桌面服务通道限制。");
            using var handle = await Task.Run(() => Connect(timeout.Token), timeout.Token).ConfigureAwait(false);
            using var stream = new FileStream(handle, FileAccess.ReadWrite, 65536, isAsync: true);
            byte[] prefix = new byte[4];
            BinaryPrimitives.WriteInt32LittleEndian(prefix, body.Length);
            await stream.WriteAsync(prefix, timeout.Token).ConfigureAwait(false);
            await stream.WriteAsync(body, timeout.Token).ConfigureAwait(false);
            await stream.FlushAsync(timeout.Token).ConfigureAwait(false);
            await stream.ReadExactlyAsync(prefix, timeout.Token).ConfigureAwait(false);
            int count = BinaryPrimitives.ReadInt32LittleEndian(prefix);
            if (count is <= 0 or > MaxFrame) throw new IOException("服务返回的状态帧大小不合法。");
            byte[] response = new byte[count];
            await stream.ReadExactlyAsync(response, timeout.Token).ConfigureAwait(false);
            await stream.WriteAsync(new byte[] { 0x06 }, timeout.Token).ConfigureAwait(false);
            await stream.FlushAsync(timeout.Token).ConfigureAwait(false);
            using var parsed = JsonDocument.Parse(response);
            var root = parsed.RootElement;
            if (root.GetProperty("version").GetUInt32() != 2 || root.GetProperty("request_id").GetInt64() != id) throw new IOException("服务响应版本或请求标识不匹配。");
            var result = root.GetProperty("result");
            if (result.ValueKind == JsonValueKind.String) throw new IOException(result.GetString() == "UnsupportedVersion" ? "客户端与服务协议版本不兼容。" : "服务未接受此操作。");
            if (result.TryGetProperty("Error", out var error) || result.TryGetProperty("Rejected", out error)) throw new IOException(error.ValueKind == JsonValueKind.String ? error.GetString() : error.String("message", "服务拒绝此操作。"));
            return result.Clone();
        }
        catch (JsonException) { throw new IOException("服务返回的状态格式不合法。"); }
        finally { requests.Release(); }
    }
    private static SafeFileHandle Connect(CancellationToken token)
    {
        while (true)
        {
            token.ThrowIfCancellationRequested();
            // FILE_GENERIC_READ | FILE_WRITE_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES.
            var handle = CreateFile(Pipe, 0x00120089 | 0x00000002 | 0x00000010 | 0x00000100, 0, IntPtr.Zero, 3,
                0x40000000 | 0x00100000 | 0x00010000, IntPtr.Zero); // OVERLAPPED + SQOS_PRESENT + IDENTIFICATION
            if (!handle.IsInvalid)
            {
                try
                {
                    if (!GetNamedPipeServerProcessId(handle, out uint pid) || pid == 0 || pid != ServiceProcessId()) throw new IOException("拒绝连接：命名管道进程不是 SCM 登记的 UniDeskService。");
                    return handle;
                }
                catch { handle.Dispose(); throw; }
            }
            int error = Marshal.GetLastWin32Error();
            handle.Dispose();
            if (error != 231) throw new IOException(error == 2 ? "后台组件正在启动，稍后自动重连。" : error == 5 ? "服务拒绝连接：当前桌面会话或客户端安装身份未获授权。" : $"连接 UniDeskService 失败（Windows {error}）。");
            WaitNamedPipe(Pipe, 500);
        }
    }
    // Query first: a running service never requires START access or elevation.
    private static void EnsureRunning(CancellationToken token)
    {
        IntPtr manager = OpenSCManager(null, null, 1);
        if (manager == IntPtr.Zero) throw new IOException("无法访问后台组件管理器。");
        try
        {
            IntPtr service = OpenService(manager, "UniDeskService", 4);
            if (service == IntPtr.Zero) throw new IOException("uniDesk 安装不完整，请重新运行完整安装包修复；无需单独启动其他程序。");
            try
            {
                long deadline = Environment.TickCount64 + 12000;
                bool attempted = false;
                while (true)
                {
                    token.ThrowIfCancellationRequested();
                    if (!QueryServiceStatusEx(service, 0, out ServiceStatus status, Marshal.SizeOf<ServiceStatus>(), out _))
                        throw new IOException("无法读取后台组件状态。");
                    if (status.CurrentState == 4) return;
                    if (status.CurrentState == 1 && !attempted)
                    {
                        if (Environment.TickCount64 - Interlocked.Read(ref lastStartAttempt) < 30000 && Interlocked.Read(ref lastStartAttempt) != 0)
                            throw new IOException(lastStartMessage);
                        attempted = true;
                        Interlocked.Exchange(ref lastStartAttempt, Environment.TickCount64);
                        try
                        {
                            IntPtr starter = OpenService(manager, "UniDeskService", 0x10);
                            if (starter == IntPtr.Zero) throw new IOException("旧版安装权限不支持自动启动，请运行新版完整安装包升级；无需手动启动后台程序。");
                            try
                            {
                                if (!StartService(starter, 0, IntPtr.Zero) && Marshal.GetLastWin32Error() != 1056)
                                    throw new IOException($"后台组件自动启动失败（Windows {Marshal.GetLastWin32Error()}）。请运行完整安装包修复。");
                                lastStartMessage = "后台组件启动后尚未保持运行，稍后自动重试。";
                            }
                            finally { CloseServiceHandle(starter); }
                        }
                        catch (IOException error) { lastStartMessage = error.Message; throw; }
                    }
                    if (Environment.TickCount64 >= deadline) throw new TimeoutException();
                    if (token.WaitHandle.WaitOne(200)) token.ThrowIfCancellationRequested();
                }
            }
            finally { CloseServiceHandle(service); }
        }
        finally { CloseServiceHandle(manager); }
    }
    private static uint ServiceProcessId()
    {
        IntPtr manager = OpenSCManager(null, null, 1);
        if (manager == IntPtr.Zero) throw new IOException("无法核对服务管理器中的服务身份。");
        try
        {
            IntPtr service = OpenService(manager, "UniDeskService", 4);
            if (service == IntPtr.Zero) throw new IOException("UniDeskService 尚未安装。");
            try
            {
                if (!QueryServiceStatusEx(service, 0, out ServiceStatus status, Marshal.SizeOf<ServiceStatus>(), out _) || status.CurrentState != 4 || status.ProcessId == 0) throw new IOException("UniDeskService 尚未就绪。");
                return status.ProcessId;
            }
            finally { CloseServiceHandle(service); }
        }
        finally { CloseServiceHandle(manager); }
    }
    public void Dispose() => stop.Cancel();
    [StructLayout(LayoutKind.Sequential)] private struct ServiceStatus { public uint ServiceType, CurrentState, ControlsAccepted, Win32ExitCode, ServiceSpecificExitCode, CheckPoint, WaitHint, ProcessId, ServiceFlags; }
    [DllImport("kernel32.dll", EntryPoint = "CreateFileW", CharSet = CharSet.Unicode, SetLastError = true)] private static extern SafeFileHandle CreateFile(string name, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool GetNamedPipeServerProcessId(SafeFileHandle pipe, out uint processId);
    [DllImport("kernel32.dll", EntryPoint = "WaitNamedPipeW", CharSet = CharSet.Unicode, SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool WaitNamedPipe(string name, uint timeout);
    [DllImport("advapi32.dll", EntryPoint = "OpenSCManagerW", CharSet = CharSet.Unicode, SetLastError = true)] private static extern IntPtr OpenSCManager(string? machine, string? database, uint access);
    [DllImport("advapi32.dll", EntryPoint = "OpenServiceW", CharSet = CharSet.Unicode, SetLastError = true)] private static extern IntPtr OpenService(IntPtr manager, string name, uint access);
    [DllImport("advapi32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool QueryServiceStatusEx(IntPtr service, int level, out ServiceStatus status, int size, out int needed);
    [DllImport("advapi32.dll", EntryPoint = "StartServiceW", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool StartService(IntPtr service, uint count, IntPtr args);
    [DllImport("advapi32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool CloseServiceHandle(IntPtr handle);
}

internal static class JsonFields
{
    public static string String(this JsonElement value, string key, string fallback = "") => value.ValueKind == JsonValueKind.Object && value.TryGetProperty(key, out var field) && field.ValueKind == JsonValueKind.String ? field.GetString() ?? fallback : fallback;
    public static bool Bool(this JsonElement value, string key) => value.ValueKind == JsonValueKind.Object && value.TryGetProperty(key, out var field) && field.ValueKind == JsonValueKind.True;
    public static long Number(this JsonElement value, string key, long fallback = 0) => value.ValueKind == JsonValueKind.Object && value.TryGetProperty(key, out var field) && field.TryGetInt64(out long number) ? number : fallback;
    public static IEnumerable<JsonElement> Array(this JsonElement value, string key) => value.ValueKind == JsonValueKind.Object && value.TryGetProperty(key, out var field) && field.ValueKind == JsonValueKind.Array ? field.EnumerateArray() : [];
}
