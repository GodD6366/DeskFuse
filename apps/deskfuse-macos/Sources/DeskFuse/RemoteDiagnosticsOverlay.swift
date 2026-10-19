import SwiftUI
import DeskFuseRemoteApplication

@available(macOS 14.2, *)
struct RemoteDiagnosticsView: View {
    let session: RemoteApplicationSession
    let close: () -> Void
    @State private var local = MediaStatistics()
    @State private var remote: RemoteMediaDiagnostics?
    @State private var fps: Double?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Button(action: close) { Image(systemName: "chevron.left").frame(width: 24, height: 24) }
                    .buttonStyle(.plain).help("返回远程应用菜单")
                Label("远程应用诊断", systemImage: "waveform.path.ecg")
                    .font(.headline)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }.padding(12)
            Divider()
            ScrollView {
                VStack(spacing: 10) {
                    metric("显示帧率", fps.map { String(format: "%.1f fps", $0) } ?? "—")
                    metric("远端编码 P95", milliseconds(remote?.media.encoded ?? 0, remote?.media.encodeP95Milliseconds))
                    metric("本机解码 P95", milliseconds(local.decoded, local.decodeP95Milliseconds))
                    metric("QUIC 往返延迟", remote.map { String(format: "%.1f ms", $0.rttMilliseconds) } ?? "—")
                    metric("远端发送丢包率", loss)
                    metric("近期丢包率（约 1 秒）", remote?.recentLossPercent.map { String(format: "%.2f%%", $0) } ?? "—")
                    metric("发送调用最长耗时", remote?.sendWaitMilliseconds.map { String(format: "%.1f ms", $0) } ?? "—")
                    metric("编码至呈现回执", remote?.media.presentationRoundTripMs.map { String(format: "%.1f ms", $0) } ?? "—")
                    metric("在途帧数", remote?.media.inFlightFrames.map(String.init) ?? "—")
                    metric("当前目标码率", remote?.media.currentBitrate.map { String(format: "%.2f Mbps", Double($0) / 1_000_000) } ?? "—")
                    Text("丢包率：共享 QUIC 连接累计；耗时：最近最多 600 帧。")
                        .font(.caption2).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Divider()
                    metric("编码前替换帧", remote.map { String($0.media.replacedBeforeEncode) } ?? "—")
                    metric("编码器丢帧", remote.map { String($0.media.encoderDropped) } ?? "—")
                    metric("解码链重置", String(local.chainResets))
                    metric("解码队列溢出", String(local.queueOverflows ?? 0))
                    metric("压缩帧序号缺口", String(local.sequenceGaps ?? 0))
                    metric("解码错误", String(local.decodeErrors ?? 0))
                    metric("关键帧请求", String(local.keyFrameRequests ?? 0))
                    metric("码率调整失败", remote?.media.bitrateUpdateFailures.map(String.init) ?? "0")
                    if let address = remote?.remoteAddress {
                        Text("远端连接到：\(address)").font(.caption).textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    Text(session.actualDescription).font(.caption).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if !session.connected || remote == nil {
                        Text(session.connected ? "等待远端统计" : "连接已断开")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(12)
            }
        }
        .task {
            var previous = session.metrics.snapshot()
            var sampledAt = ProcessInfo.processInfo.systemUptime
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                let now = ProcessInfo.processInfo.systemUptime
                let next = session.metrics.snapshot()
                fps = next.presented >= previous.presented && now > sampledAt
                    ? Double(next.presented - previous.presented) / (now - sampledAt) : nil
                local = next
                remote = session.connected ? session.plane.diagnosticSnapshot() : nil
                previous = next; sampledAt = now
            }
        }
    }

    private var loss: String {
        guard let remote, remote.sentPackets > 0 else { return "—" }
        return String(format: "%.2f%%", min(100, Double(remote.lostPackets) / Double(remote.sentPackets) * 100))
    }
    private func milliseconds(_ count: UInt64, _ value: Double?) -> String {
        guard count > 0, let value, value.isFinite else { return "—" }
        return String(format: "%.2f ms", value)
    }
    private func metric(_ name: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(name).foregroundStyle(.secondary)
            Spacer(minLength: 8)
            Text(value).monospacedDigit().textSelection(.enabled)
        }.font(DFType.detail)
    }
}
