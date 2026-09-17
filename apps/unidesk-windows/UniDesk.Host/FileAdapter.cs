namespace UniDesk.Host;

public sealed class WindowsFileAuthorization
{
    public IReadOnlyList<string> AuthorizeRead(IEnumerable<string> paths) =>
        paths.Where(File.Exists).ToArray();

    public string AuthorizeWrite(string inboxDirectory)
    {
        Directory.CreateDirectory(inboxDirectory);
        return inboxDirectory;
    }
}
