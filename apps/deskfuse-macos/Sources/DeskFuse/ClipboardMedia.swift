import Foundation
import ImageIO
import UniformTypeIdentifiers
import DeskFuseEngine

/// Derived preview data only. Business state and original formats stay in Rust.
struct ClipboardMedia: Sendable {
    let thumbnail: Data?
    let localPaths: Set<String>
    let pixelWidth: Int?
    let pixelHeight: Int?
    var firstFileIsDirectory = false
}

actor ClipboardMediaLoader {
    static let shared = ClipboardMediaLoader()

    func load(_ content: ClipboardContent, maxPixelSize: Int = 2048) -> ClipboardMedia {
        let localPaths = Set(content.filePaths.filter { FileManager.default.fileExists(atPath: $0) })
        var isDirectory: ObjCBool = false
        if let firstPath = content.filePaths.first {
            _ = FileManager.default.fileExists(atPath: firstPath, isDirectory: &isDirectory)
        }
        let options = [kCGImageSourceShouldCache: false] as CFDictionary
        let source: CGImageSource?
        if !content.imagePng.isEmpty {
            source = CGImageSourceCreateWithData(content.imagePng as CFData, options)
        } else if content.filePaths.count == 1, let path = content.filePaths.first, localPaths.contains(path),
                  UTType(filenameExtension: URL(fileURLWithPath: path).pathExtension)?.conforms(to: .image) == true {
            // Read local previews only, without fetching remote file references.
            source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, options)
        } else {
            source = nil
        }
        guard let source else {
            return ClipboardMedia(thumbnail: nil, localPaths: localPaths, pixelWidth: nil, pixelHeight: nil, firstFileIsDirectory: isDirectory.boolValue)
        }
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        let width = (properties?[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue
        let height = (properties?[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue
        // Bound decode size and keep ImageIO work off the main actor. Never modify
        // or transcode the original payload restored by the engine's copy command.
        let thumbnailOptions: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: max(64, min(maxPixelSize, 2048)),
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnailOptions as CFDictionary) else {
            return ClipboardMedia(thumbnail: nil, localPaths: localPaths, pixelWidth: width, pixelHeight: height, firstFileIsDirectory: isDirectory.boolValue)
        }
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil) else {
            return ClipboardMedia(thumbnail: nil, localPaths: localPaths, pixelWidth: width, pixelHeight: height, firstFileIsDirectory: isDirectory.boolValue)
        }
        CGImageDestinationAddImage(destination, image, nil)
        let thumbnail = CGImageDestinationFinalize(destination) ? data as Data : nil
        return ClipboardMedia(thumbnail: thumbnail, localPaths: localPaths, pixelWidth: width, pixelHeight: height, firstFileIsDirectory: isDirectory.boolValue)
    }
}
