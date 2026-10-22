using System.Reflection;

namespace DeskFuse.Windows;

internal static class AppVersion
{
    public static string Display
    {
        get
        {
            var value = typeof(AppVersion).Assembly.GetCustomAttribute<AssemblyFileVersionAttribute>()?.Version;
            return Version.TryParse(value, out var version)
                ? $"{version.Major}.{version.Minor}.{version.Build}"
                : "—";
        }
    }
}
