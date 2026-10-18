import Foundation
import AppKit
import AVFoundation
import CoreAudio
import AudioToolbox
import Darwin
import MicrophoneNative

struct MicrophoneReader: Decodable, Identifiable, Sendable {
    let id: UInt32
    let name: String
    let isSoundSettings: Bool
}
struct MicrophoneInput: Identifiable, Sendable { let id: String; let name: String }
enum MicrophoneAudioDevices {
    static let virtualUID = String(cString: ud_mic_device_uid())
    static func string(_ id: AudioObjectID, _ selector: AudioObjectPropertySelector) -> String? {
        var address = AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var value: Unmanaged<CFString>?; var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        let status = withUnsafeMutablePointer(to: &value) { AudioObjectGetPropertyData(id, &address, 0, nil, &size, $0) }
        guard status == noErr, let value else { return nil }
        return value.takeRetainedValue() as String
    }
    static func devices() -> [(AudioDeviceID, String, String)] {
        var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size) == noErr else { return [] }
        var ids = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &ids) == noErr else { return [] }
        return ids.compactMap { id in
            var streams = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreams, mScope: kAudioDevicePropertyScopeInput, mElement: kAudioObjectPropertyElementMain)
            var bytes: UInt32 = 0
            guard AudioObjectGetPropertyDataSize(id, &streams, 0, nil, &bytes) == noErr, bytes > 0,
                  let uid = string(id, kAudioDevicePropertyDeviceUID) else { return nil }
            return (id, uid, string(id, kAudioObjectPropertyName) ?? uid)
        }
    }
    /// Read-only attribution. This never opens an input stream and is not used
    /// to gate audio: older macOS versions and unidentifiable clients still work.
    static func activeReaders() -> [MicrophoneReader] {
        guard #available(macOS 14.2, *),
              let device = devices().first(where: { $0.1 == virtualUID })?.0 else { return [] }
        return integers(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyProcessObjectList).compactMap { process in
            guard integers(process, kAudioProcessPropertyIsRunningInput).first == 1,
                  integers(process, kAudioProcessPropertyDevices, scope: kAudioObjectPropertyScopeInput).contains(device),
                  let pid = integers(process, kAudioProcessPropertyPID).first else { return nil }
            let bundleID = string(process, kAudioProcessPropertyBundleID) ?? ""
            let soundSettings = bundleID == "com.apple.Sound-Settings.extension"
            let name = soundSettings ? "系统声音设置" : (NSRunningApplication(processIdentifier: pid_t(pid))?.localizedName ?? "音频应用")
            return MicrophoneReader(id: pid, name: name, isSoundSettings: soundSettings)
        }.sorted { $0.id < $1.id }
    }
    private static func integers(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector,
                                 scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> [UInt32] {
        var address = AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(object, &address, 0, nil, &size) == noErr, size > 0 else { return [] }
        var values = [UInt32](repeating: 0, count: Int(size) / MemoryLayout<UInt32>.size)
        guard AudioObjectGetPropertyData(object, &address, 0, nil, &size, &values) == noErr else { return [] }
        return Array(values.prefix(Int(size) / MemoryLayout<UInt32>.size))
    }
    static func isPhysical(_ id: AudioDeviceID, uid: String) -> Bool {
        if uid == virtualUID { return false }
        var address = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyTransportType, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var transport: UInt32 = 0; var size = UInt32(MemoryLayout<UInt32>.size)
        guard AudioObjectGetPropertyData(id, &address, 0, nil, &size, &transport) == noErr else { return false }
        return transport != kAudioDeviceTransportTypeVirtual && transport != kAudioDeviceTransportTypeAggregate
    }
    static func physicalInputs() -> [MicrophoneInput] {
        devices().filter { id, uid, _ in isPhysical(id, uid: uid) }
            .map { MicrophoneInput(id: $0.1, name: $0.2) }
    }
    static func defaultInput() -> AudioDeviceID? {
        var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultInputDevice, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var id: AudioDeviceID = 0; var size = UInt32(MemoryLayout<AudioDeviceID>.size)
        return AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &id) == noErr ? id : nil
    }
    static func setDefault(uid: String) throws {
        guard var id = devices().first(where: { $0.1 == uid })?.0 else { throw MicrophoneAudioError.message("找不到所选麦克风") }
        var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultInputDevice, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        guard AudioObjectSetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, UInt32(MemoryLayout<AudioDeviceID>.size), &id) == noErr else {
            throw MicrophoneAudioError.message("无法设置系统默认输入")
        }
    }
}
enum MicrophoneAudioError: LocalizedError {
    case message(String)
    var errorDescription: String? { if case .message(let text) = self { return text }; return nil }
}
/// The tap only converts and deposits a bounded batch; encoding/network run on
/// the audio worker. If the worker stalls, close the session rather than queue speech.
final class MicrophoneCapture: @unchecked Sendable {
    private let audio = AVAudioEngine()
    private let lock = NSLock()
    private var frames: [(UInt64, [Float])] = []
    private var overflow = false
    private var captureError: String?
    private var inputFrames: UInt64 = 0
    private var peak: Float = 0
    private var tapped = false
    private var device: AudioDeviceID = 0
    func start(uid: String) throws {
        guard AVCaptureDevice.authorizationStatus(for: .audio) == .authorized else { throw MicrophoneAudioError.message("等待麦克风授权") }
        let devices = MicrophoneAudioDevices.devices().filter { MicrophoneAudioDevices.isPhysical($0.0, uid: $0.1) }
        let selected = uid.isEmpty ? devices.first(where: { $0.0 == MicrophoneAudioDevices.defaultInput() }) ?? devices.first : devices.first(where: { $0.1 == uid })
        guard let selected else { throw MicrophoneAudioError.message("所选实体麦克风不可用，请在设置中重新选择") }
        device = selected.0
        let input = audio.inputNode
        guard let unit = input.audioUnit,
              AudioUnitSetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &device, UInt32(MemoryLayout<AudioDeviceID>.size)) == noErr else {
            throw MicrophoneAudioError.message("无法打开所选实体麦克风")
        }
        // The output bus may still cache the previous device format after
        // CurrentDevice changes. A tap must match the physical input format.
        let format = input.inputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0,
              let target = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 1, interleaved: false),
              let converter = AVAudioConverter(from: format, to: target) else { throw MicrophoneAudioError.message("麦克风格式不受支持") }
        converter.downmix = true
        input.installTap(onBus: 0, bufferSize: 480, format: format) { [weak self] buffer, _ in
            guard let self else { return }
            self.lock.lock(); self.inputFrames += UInt64(buffer.frameLength); self.lock.unlock()
            let capacity = AVAudioFrameCount(ceil(Double(buffer.frameLength) * 48_000 / format.sampleRate)) + 32
            guard let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { return }
            var consumed = false; var error: NSError?
            let status = converter.convert(to: output, error: &error) { _, status in
                if consumed { status.pointee = .noDataNow; return nil }
                consumed = true; status.pointee = .haveData; return buffer
            }
            if status == .error || error != nil {
                self.lock.lock()
                self.captureError = "麦克风格式转换失败：\(error?.localizedDescription ?? "未知转换错误")"
                self.lock.unlock()
                return
            }
            guard output.frameLength > 0, let samples = output.floatChannelData?[0] else { return }
            let values = Array(UnsafeBufferPointer(start: samples, count: Int(output.frameLength)))
            self.lock.lock()
            if self.frames.count < 8 { self.frames.append((DispatchTime.now().uptimeNanoseconds, values)) } else { self.overflow = true }
            self.peak = values.reduce(Float(0)) { max($0, abs($1)) }
            self.lock.unlock()
        }
        tapped = true
        audio.prepare()
        do { try audio.start() } catch { stop(); throw error }
    }
    func take() throws -> [[Float]] {
        lock.lock(); defer { lock.unlock() }
        if let captureError { throw MicrophoneAudioError.message(captureError) }
        guard !overflow, audio.isRunning else { throw MicrophoneAudioError.message("麦克风采集中断或音频队列积压") }
        let now = DispatchTime.now().uptimeNanoseconds
        guard frames.allSatisfy({ now >= $0.0 && now - $0.0 < 100_000_000 }) else { throw MicrophoneAudioError.message("已丢弃过期采集音频") }
        let values = frames.map { $0.1 }; frames.removeAll(keepingCapacity: true); return values
    }
    var timeoutReason: String {
        lock.lock(); defer { lock.unlock() }
        return inputFrames == 0 ? "实体麦克风未返回采集帧" : "实体麦克风格式转换未产生音频（输入帧：\(inputFrames)）"
    }
    var level: Float { lock.lock(); defer { lock.unlock() }; return peak }
    func stop() {
        audio.stop()
        if tapped { audio.inputNode.removeTap(onBus: 0); tapped = false }
        lock.lock(); frames.removeAll(); overflow = false; peak = 0; captureError = nil; inputFrames = 0; lock.unlock()
    }
    deinit { stop() }
}
/// A loopback-only bridge. Each fresh session gets fresh ephemeral source ports,
/// so feedback from an old receiver session cannot reach a new sender.
final class MicrophoneLoopback {
    private var sockets: [Int32] = []
    init() throws {
        do {
            for port in [19721, 19722, 19723] {
                let fd = Darwin.socket(AF_INET, SOCK_DGRAM, 0)
                guard fd >= 0 else { throw MicrophoneAudioError.message("无法创建音频回环通道") }
                sockets.append(fd)
                _ = fcntl(fd, F_SETFL, O_NONBLOCK)
                var address = sockaddr_in(); address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
                address.sin_family = sa_family_t(AF_INET); address.sin_port = UInt16(port).bigEndian
                address.sin_addr.s_addr = inet_addr("127.0.0.1")
                let result = withUnsafePointer(to: &address) { pointer in pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) } }
                guard result == 0 else { throw MicrophoneAudioError.message("无法连接虚拟麦克风接收器") }
            }
        } catch { sockets.forEach { Darwin.close($0) }; sockets.removeAll(); throw error }
    }
    func send(_ packet: Data) throws {
        guard let kind = packet.first, (1...3).contains(kind), packet.count > 1 else { throw MicrophoneAudioError.message("音频包格式无效") }
        let bytes = packet.dropFirst()
        let count = bytes.withUnsafeBytes { Darwin.send(sockets[Int(kind) - 1], $0.baseAddress, $0.count, 0) }
        guard count == bytes.count else { throw MicrophoneAudioError.message("虚拟麦克风接收器未就绪") }
    }
    func feedback() -> Data? {
        var bytes = [UInt8](repeating: 0, count: 4096)
        let count = Darwin.recv(sockets[2], &bytes, bytes.count, 0)
        guard count > 0 else { return nil }
        return Data([3] + bytes.prefix(count))
    }
    deinit { sockets.forEach { Darwin.close($0) } }
}
