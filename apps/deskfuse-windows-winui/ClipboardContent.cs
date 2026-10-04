using System.Security.Cryptography;
using System.Text;

namespace DeskFuse.Windows;

public sealed record ClipboardRepresentation(string Mime, byte[] Data);

public sealed record ClipboardContent(string Mime, byte[] Data,
    IReadOnlyList<string>? Files = null,
    IReadOnlyList<ClipboardRepresentation>? Representations = null)
{
    public const string LocalFilesMime = "application/x-deskfuse-local-files";
    public long SizeBytes => Data.LongLength + (Representations?.Sum(r => r.Data.LongLength) ?? 0);
    public string Fingerprint()
    {
        using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
        void Add(byte[] bytes) { hash.AppendData(BitConverter.GetBytes(bytes.Length)); hash.AppendData(bytes); }
        if (Files is not null)
        {
            Add(Encoding.UTF8.GetBytes(LocalFilesMime));
            foreach (var file in Files) Add(Encoding.UTF8.GetBytes(file));
        }
        else
        {
            // Primary is a view of one representation, not an extra format.
            // Stable order preserves rich formats while ignoring wire enumeration order.
            var representations = (Representations ?? Array.Empty<ClipboardRepresentation>())
                .Append(new ClipboardRepresentation(Mime, Data))
                .GroupBy(r => (r.Mime, Hash: Convert.ToHexString(SHA256.HashData(r.Data))))
                .Select(g => g.First()).OrderBy(r => r.Mime, StringComparer.Ordinal)
                .ThenBy(r => Convert.ToHexString(SHA256.HashData(r.Data)), StringComparer.Ordinal);
            foreach (var rep in representations) { Add(Encoding.UTF8.GetBytes(rep.Mime)); Add(rep.Data); }
        }
        return Convert.ToHexString(hash.GetHashAndReset());
    }
}
