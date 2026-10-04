using Microsoft.UI.Dispatching;

namespace DeskFuse.Windows;

/// <summary>Awaitable DispatcherQueue hop; WinUI 3 has no InvokeAsync equivalent.</summary>
public static class DispatcherQueueExtensions
{
    public static Task EnqueueAsync(this DispatcherQueue dispatcher, Action action)
    {
        var completion = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        if (!dispatcher.TryEnqueue(() =>
            {
                try { action(); completion.SetResult(); }
                catch (Exception error) { completion.SetException(error); }
            }))
        {
            completion.SetException(new InvalidOperationException("dispatcher queue rejected work"));
        }
        return completion.Task;
    }
}
