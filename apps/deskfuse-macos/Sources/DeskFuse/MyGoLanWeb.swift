import AppKit
import CoreImage
import CoreImage.CIFilterBuiltins
import Foundation

/// 局域网访问地址的二维码。原生页面用 CoreImage 生成；WebView 页面通过桥接
/// 取同一份位图，避免在前端引入二维码编码依赖。
enum MyGoLanWeb {
    static func qrCodeDataURL(for address: String) -> String? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(address.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
              let image = CIContext().createCGImage(output, from: output.extent) else { return nil }
        let representation = NSBitmapImageRep(cgImage: image)
        guard let png = representation.representation(using: .png, properties: [:]) else { return nil }
        return "data:image/png;base64," + png.base64EncodedString()
    }
}
