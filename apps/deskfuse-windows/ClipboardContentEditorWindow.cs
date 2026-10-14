using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Text;

namespace DeskFuse.Windows;

/// <summary>Editing creates a new clipboard capture; the original history entry stays intact.</summary>
internal sealed class ClipboardContentEditorWindow
{
    private readonly Window window = new();
    private readonly RichEditBox editor = new() { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap };
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap };
    private bool plainOnly;
    public event Action? Closed;

    public ClipboardContentEditorWindow(ClipboardEntry entry, Action<ClipboardContent> save)
    {
        AppIcon.Apply(window);
        window.AppWindow.Title = "编辑剪贴板内容";
        var id = Microsoft.UI.Win32Interop.GetWindowIdFromWindow(WinRT.Interop.WindowNative.GetWindowHandle(window));
        var area = Microsoft.UI.Windowing.DisplayArea.GetFromWindowId(id, Microsoft.UI.Windowing.DisplayAreaFallback.Nearest).WorkArea;
        window.AppWindow.Resize(new global::Windows.Graphics.SizeInt32(Math.Min(760, area.Width), Math.Min(560, area.Height)));
        window.Closed += (_, _) => Closed?.Invoke();
        var root = new Grid { Padding = new Thickness(20), RowSpacing = 12 };
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var toolbar = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        Button StyleButton(string label, Action action)
        {
            var button = new Button { Content = label };
            button.Click += (_, _) => { action(); editor.Focus(FocusState.Programmatic); };
            toolbar.Children.Add(button);
            return button;
        }
        var bold = StyleButton("粗体", () => editor.Document.Selection.CharacterFormat.Bold = FormatEffect.Toggle);
        var italic = StyleButton("斜体", () => editor.Document.Selection.CharacterFormat.Italic = FormatEffect.Toggle);
        var underline = StyleButton("下划线", () =>
        {
            var format = editor.Document.Selection.CharacterFormat;
            format.Underline = format.Underline == UnderlineType.Single ? UnderlineType.None : UnderlineType.Single;
        });
        var strike = StyleButton("删除线", () => editor.Document.Selection.CharacterFormat.Strikethrough = FormatEffect.Toggle);
        var plain = new ToggleSwitch { Header = "仅纯文本", IsOn = false };
        plain.Toggled += (_, _) =>
        {
            plainOnly = plain.IsOn;
            if (plainOnly) editor.Document.SetText(TextSetOptions.None, PlainText());
            foreach (var button in new[] { bold, italic, underline, strike }) button.IsEnabled = !plainOnly;
        };
        toolbar.Children.Add(plain);
        var saveButton = new Button { Content = "保存", Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
        saveButton.Click += (_, _) =>
        {
            try
            {
                byte[] text = Encoding.UTF8.GetBytes(PlainText());
                var formats = new List<ClipboardRepresentation> { new("text/plain", text) };
                if (!plainOnly)
                {
                    editor.Document.GetText(TextGetOptions.FormatRtf, out string rtf);
                    formats.Add(new("text/rtf", Encoding.UTF8.GetBytes(rtf)));
                }
                if (formats.Sum(format => (long)format.Data.Length) > NativeClipboard.MaxContentBytes)
                {
                    status.Text = "编辑后的内容过大，请减少内容后再保存。";
                    return;
                }
                save(new ClipboardContent("text/plain", text, Representations: formats));
                window.Close();
            }
            catch (Exception ex) when (ex is IOException or InvalidOperationException or ArgumentException or COMException)
            { status.Text = "保存未完成：" + ex.Message; }
        };
        toolbar.Children.Add(saveButton);
        root.Children.Add(toolbar);
        Grid.SetRow(editor, 1); root.Children.Add(editor);
        Grid.SetRow(status, 2); root.Children.Add(status);
        window.Content = root;
        // Like the Mac editor, begin with the searchable text and let the user apply styles.
        editor.Document.SetText(TextSetOptions.None, entry.Preview);
    }

    private string PlainText()
    {
        editor.Document.GetText(TextGetOptions.None, out string text);
        // RichEdit exposes its mandatory final paragraph marker. Remove that one
        // marker only, preserving explicit trailing empty paragraphs in the draft.
        return text.EndsWith('\r') ? text[..^1] : text;
    }

    public void Show() { window.AppWindow.Show(); window.Activate(); }
    public void Close() => window.Close();
}
