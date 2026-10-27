using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;

namespace DeskFuse.Windows;

/// macOS parity: the brand mark uses the light artwork on a light surface and
/// the dark artwork on a dark surface. The executable and tray icon keep the
/// light artwork, matching the macOS application icon.
internal static class BrandMark
{
    private static readonly Uri Light = new("ms-appx:///Assets/BrandMark.Light.png");
    private static readonly Uri Dark = new("ms-appx:///Assets/BrandMark.Dark.png");

    internal static void Apply(Image image, FrameworkElement themeSource)
    {
        void Update() => image.Source = new BitmapImage(themeSource.ActualTheme == ElementTheme.Dark ? Dark : Light);
        Update();
        themeSource.ActualThemeChanged += (_, _) => Update();
        themeSource.Loaded += (_, _) => Update();
    }
}
