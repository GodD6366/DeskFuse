using Microsoft.Win32;

namespace DeskFuse.Windows;

/// Per-user logon registration. Startup never elevates or changes the machine service.
internal static class StartupRegistration
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "DeskFuse";

    public static bool IsEnabled
    {
        get
        {
            using var key = Registry.CurrentUser.OpenSubKey(RunKey);
            return key?.GetValue(ValueName) is string command && command == ExpectedCommand;
        }
    }

    public static void SetEnabled(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(RunKey);
        if (enabled) key.SetValue(ValueName, ExpectedCommand, RegistryValueKind.String);
        else key.DeleteValue(ValueName, false);
    }

    private static string ExpectedCommand => $"\"{Environment.ProcessPath}\" --minimized";
}
