import Foundation

struct AuthorizedFile {
    var url: URL
    var size: UInt64
}

final class MacOSFileAuthorization {
    func authorizeRead(urls: [URL]) throws -> [AuthorizedFile] {
        try urls.map { url in
            let values = try url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
            guard values.isRegularFile == true else {
                throw CocoaError(.fileReadUnsupportedScheme)
            }
            return AuthorizedFile(url: url, size: UInt64(values.fileSize ?? 0))
        }
    }

    func authorizeWrite(names: [String], totalSize: UInt64) throws -> URL {
        let _ = names
        let _ = totalSize
        return FileManager.default.temporaryDirectory.appendingPathComponent("unidesk-inbox", isDirectory: true)
    }
}
